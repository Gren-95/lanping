//! Works out what a service speaks when it is not TLS: reads what it sends first, and if it
//! sends nothing, asks it a plain HTTP question and reads the answer.

use std::fmt;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::socket_budget::socket_permit;

const CONNECT_WAIT: Duration = Duration::from_secs(3);
const READ_WAIT: Duration = Duration::from_millis(1500);
const READ_LIMIT: usize = 1024;
const MAX_SHOWN_CHARS: usize = 80;
const MYSQL_PROTOCOL_VERSION: u8 = 10;

/// What a port answered with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlainService {
    Http {
        status: String,
        server: Option<String>,
        /// Where a redirect points, which often is the HTTPS address.
        location: Option<String>,
    },
    Ssh(String),
    Smtp(String),
    Ftp(String),
    SmtpOrFtp(String),
    Pop3(String),
    Imap(String),
    RedisOrPop3(String),
    MySql(String),
    Rtsp(String),
    Smb,
    /// Readable text that matched nothing above.
    Text(String),
    /// Bytes that are not readable text.
    Binary(usize),
    /// Connected and sent nothing, even when asked.
    Silent,
}

/// A sentence fragment that follows "This port is": a service with its details.
impl fmt::Display for PlainService {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http {
                status,
                server,
                location,
            } => {
                write!(f, "a plain HTTP server ({status}")?;
                if let Some(server) = server {
                    write!(f, "; Server: {server}")?;
                }
                if let Some(location) = location {
                    write!(f, "; redirects to {location}")?;
                }
                f.write_str(")")
            }
            Self::Ssh(banner) => write!(f, "an SSH server ({banner})"),
            Self::Smtp(banner) => write!(f, "an SMTP mail server ({banner})"),
            Self::Ftp(banner) => write!(f, "an FTP server ({banner})"),
            Self::SmtpOrFtp(banner) => write!(f, "an SMTP or FTP server ({banner})"),
            Self::Pop3(banner) => write!(f, "a POP3 mail server ({banner})"),
            Self::Imap(banner) => write!(f, "an IMAP mail server ({banner})"),
            Self::RedisOrPop3(reply) => write!(f, "a Redis or POP3 server (it replied {reply})"),
            Self::MySql(version) => write!(f, "a MySQL or MariaDB server (version {version})"),
            Self::Rtsp(banner) => write!(f, "an RTSP media server ({banner})"),
            Self::Smb => f.write_str("an SMB file server"),
            Self::Text(line) => write!(f, "something that is not TLS (\"{line}\")"),
            Self::Binary(length) => {
                write!(
                    f,
                    "something that is not TLS ({length} bytes of binary data)"
                )
            }
            Self::Silent => f.write_str("a service that sent nothing"),
        }
    }
}

/// What a port number is usually used for, for services that stay silent until they are
/// spoken to in their own protocol.
pub fn usual_use_of_port(port: u16) -> Option<&'static str> {
    match port {
        21 => Some("FTP"),
        23 => Some("Telnet"),
        25 | 465 | 587 => Some("mail submission (SMTP)"),
        53 => Some("DNS"),
        110 | 995 => Some("POP3 mail"),
        139 => Some("NetBIOS file sharing"),
        143 | 993 => Some("IMAP mail"),
        445 => Some("SMB file sharing"),
        1883 => Some("MQTT"),
        3306 => Some("MySQL or MariaDB"),
        3389 => Some("Remote Desktop (RDP)"),
        5432 => Some("PostgreSQL"),
        5900 => Some("VNC"),
        6379 => Some("Redis"),
        27017 => Some("MongoDB"),
        _ => None,
    }
}

/// Connects to `host:port` and works out what it speaks. `name` is the host name to give
/// when asking an HTTP question.
pub async fn identify(host: &str, port: u16, name: &str) -> PlainService {
    let _permit = socket_permit().await;
    let Ok(Ok(mut stream)) = timeout(CONNECT_WAIT, TcpStream::connect((host, port))).await else {
        return PlainService::Silent;
    };
    let first = read_some(&mut stream).await;
    if !first.is_empty() {
        return classify(&first);
    }
    let question = format!("GET / HTTP/1.0\r\nHost: {name}\r\nUser-Agent: lanping\r\n\r\n");
    if timeout(READ_WAIT, stream.write_all(question.as_bytes()))
        .await
        .is_err()
    {
        return PlainService::Silent;
    }
    classify(&read_some(&mut stream).await)
}

/// Reads what arrives within a short wait. Empty when nothing does.
async fn read_some(stream: &mut TcpStream) -> Vec<u8> {
    let mut buffer = vec![0; READ_LIMIT];
    match timeout(READ_WAIT, stream.read(&mut buffer)).await {
        Ok(Ok(length)) => {
            buffer.truncate(length);
            buffer
        }
        _ => Vec::new(),
    }
}

/// Names the service that sent `bytes`.
pub fn classify(bytes: &[u8]) -> PlainService {
    if bytes.is_empty() {
        return PlainService::Silent;
    }
    if let Some(version) = mysql_version(bytes) {
        return PlainService::MySql(version);
    }
    if is_smb(bytes) {
        return PlainService::Smb;
    }
    let text = String::from_utf8_lossy(bytes);
    let first_line = shown(text.lines().next().unwrap_or_default());
    if text.starts_with("HTTP/") {
        return http(&text);
    }
    let lower = first_line.to_lowercase();
    if first_line.starts_with("SSH-") {
        PlainService::Ssh(first_line)
    } else if first_line.starts_with("RTSP/") {
        PlainService::Rtsp(first_line)
    } else if first_line.starts_with("220") {
        if lower.contains("ftp") {
            PlainService::Ftp(first_line)
        } else if lower.contains("smtp") {
            PlainService::Smtp(first_line)
        } else {
            PlainService::SmtpOrFtp(first_line)
        }
    } else if first_line.starts_with("+OK") {
        PlainService::Pop3(first_line)
    } else if first_line.starts_with("* OK") || first_line.starts_with("* PREAUTH") {
        PlainService::Imap(first_line)
    } else if first_line.starts_with("-ERR") || first_line.starts_with("-DENIED") {
        PlainService::RedisOrPop3(first_line)
    } else if first_line.is_empty() {
        PlainService::Binary(bytes.len())
    } else {
        PlainService::Text(first_line)
    }
}

fn http(response: &str) -> PlainService {
    let header = |wanted: &str| {
        response.lines().skip(1).find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.trim()
                .eq_ignore_ascii_case(wanted)
                .then(|| shown(value))
                .filter(|value| !value.is_empty())
        })
    };
    PlainService::Http {
        status: shown(response.lines().next().unwrap_or_default()),
        server: header("server"),
        location: header("location"),
    }
}

/// The version string of a MySQL or MariaDB greeting: three length bytes, sequence number 0,
/// protocol version 10, then the server version ending in a zero byte.
fn mysql_version(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 6 || bytes[3] != 0 || bytes[4] != MYSQL_PROTOCOL_VERSION {
        return None;
    }
    let version = bytes[5..].split(|byte| *byte == 0).next()?;
    let text = std::str::from_utf8(version).ok()?;
    let looks_like_version = text.chars().next().is_some_and(|c| c.is_ascii_digit())
        && text.chars().all(|c| c.is_ascii_graphic());
    looks_like_version.then(|| text.to_owned())
}

/// An SMB message follows a four byte NetBIOS header with `\xFFSMB` or `\xFESMB`.
fn is_smb(bytes: &[u8]) -> bool {
    bytes.len() >= 8 && matches!(bytes[4..8], [0xFF | 0xFE, b'S', b'M', b'B'])
}

/// One short printable line.
fn shown(text: &str) -> String {
    let printable: String = text
        .chars()
        .filter(|character| character.is_ascii_graphic() || *character == ' ')
        .take(MAX_SHOWN_CHARS)
        .collect();
    printable.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn recognizes_http_with_server_and_redirect() {
        let reply = b"HTTP/1.1 308 Permanent Redirect\r\nLocation: https://192.168.0.200/\r\nServer: Caddy\r\n\r\n";
        let service = classify(reply);
        assert_eq!(
            service,
            PlainService::Http {
                status: "HTTP/1.1 308 Permanent Redirect".to_owned(),
                server: Some("Caddy".to_owned()),
                location: Some("https://192.168.0.200/".to_owned()),
            }
        );
        assert_eq!(
            service.to_string(),
            "a plain HTTP server (HTTP/1.1 308 Permanent Redirect; Server: Caddy; redirects to https://192.168.0.200/)"
        );
    }

    #[test]
    fn recognizes_http_without_extra_headers() {
        let service = classify(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
        assert_eq!(
            service.to_string(),
            "a plain HTTP server (HTTP/1.1 400 Bad Request)"
        );
    }

    #[test]
    fn recognizes_servers_that_speak_first() {
        assert_eq!(
            classify(b"SSH-2.0-OpenSSH_10.0p2 Debian-7\r\n"),
            PlainService::Ssh("SSH-2.0-OpenSSH_10.0p2 Debian-7".to_owned())
        );
        assert!(matches!(
            classify(b"220 mail.test ESMTP Postfix\r\n"),
            PlainService::Smtp(_)
        ));
        assert!(matches!(
            classify(b"220 (vsFTPd 3.0.5)\r\n"),
            PlainService::Ftp(_)
        ));
        assert!(matches!(
            classify(b"220 ready\r\n"),
            PlainService::SmtpOrFtp(_)
        ));
        assert!(matches!(
            classify(b"+OK Dovecot ready.\r\n"),
            PlainService::Pop3(_)
        ));
        assert!(matches!(
            classify(b"* OK IMAP4rev1 ready\r\n"),
            PlainService::Imap(_)
        ));
        assert!(matches!(
            classify(b"-ERR unknown command\r\n"),
            PlainService::RedisOrPop3(_)
        ));
        assert!(matches!(
            classify(b"RTSP/1.0 400 Bad Request\r\n"),
            PlainService::Rtsp(_)
        ));
    }

    #[test]
    fn recognizes_a_mysql_greeting() {
        let mut greeting = vec![0x4a, 0, 0, 0, 10];
        greeting.extend(b"8.0.36-0ubuntu0.22.04.1\0");
        greeting.extend([1, 2, 3, 4]);
        assert_eq!(
            classify(&greeting),
            PlainService::MySql("8.0.36-0ubuntu0.22.04.1".to_owned())
        );
    }

    #[test]
    fn knows_what_common_ports_are_for() {
        assert_eq!(usual_use_of_port(1883), Some("MQTT"));
        assert_eq!(usual_use_of_port(445), Some("SMB file sharing"));
        assert_eq!(usual_use_of_port(50000), None);
    }

    #[test]
    fn recognizes_an_smb_reply() {
        let reply = [0, 0, 0, 0x40, 0xFE, b'S', b'M', b'B', 0, 0];
        assert_eq!(classify(&reply), PlainService::Smb);
    }

    #[test]
    fn describes_unknown_answers() {
        assert_eq!(
            classify(b"hello there\n"),
            PlainService::Text("hello there".to_owned())
        );
        assert_eq!(classify(&[0x01, 0x02, 0x03]), PlainService::Binary(3));
        assert_eq!(classify(b""), PlainService::Silent);
    }

    #[test]
    fn long_and_unprintable_text_is_shortened() {
        let long = "a".repeat(300);
        let PlainService::Text(line) = classify(long.as_bytes()) else {
            panic!("expected text");
        };
        assert_eq!(line.len(), MAX_SHOWN_CHARS);
    }

    async fn serve_once(reply: Option<&'static [u8]>, wait_for_question: bool) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                tokio::spawn(async move {
                    if wait_for_question {
                        let mut buffer = [0; 256];
                        let _ = stream.read(&mut buffer).await;
                    }
                    if let Some(reply) = reply {
                        let _ = stream.write_all(reply).await;
                    }
                    tokio::time::sleep(Duration::from_secs(3)).await;
                });
            }
        });
        port
    }

    #[tokio::test]
    async fn identifies_a_server_that_speaks_first() {
        let port = serve_once(Some(b"SSH-2.0-test\r\n"), false).await;
        assert_eq!(
            identify("127.0.0.1", port, "x").await,
            PlainService::Ssh("SSH-2.0-test".to_owned())
        );
    }

    #[tokio::test]
    async fn asks_an_http_question_of_a_server_that_waits() {
        let port = serve_once(Some(b"HTTP/1.0 200 OK\r\nServer: test\r\n\r\n"), true).await;
        let service = identify("127.0.0.1", port, "x").await;
        assert!(matches!(service, PlainService::Http { server: Some(ref s), .. } if s == "test"));
    }

    #[tokio::test]
    async fn a_server_that_never_answers_is_silent() {
        let port = serve_once(None, true).await;
        assert_eq!(identify("127.0.0.1", port, "x").await, PlainService::Silent);
    }
}
