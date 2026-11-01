//! Reads the greeting a service sends after connecting, such as an SSH or
//! SMTP version line or an HTTP `Server` header.

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const READ_LIMIT: usize = 512;
const MAX_BANNER_CHARS: usize = 100;
const HTTP_PORTS: [u16; 6] = [80, 8000, 8008, 8080, 8888, 9000];
/// TLS servers wait for the client to speak first, so they give no banner.
const TLS_PORTS: [u16; 6] = [443, 636, 993, 995, 8443, 8883];

pub async fn grab_banner(ip: Ipv4Addr, port: u16, wait: Duration) -> Option<String> {
    if TLS_PORTS.contains(&port) {
        return None;
    }
    let mut stream = timeout(wait, TcpStream::connect((ip, port)))
        .await
        .ok()?
        .ok()?;
    if HTTP_PORTS.contains(&port) {
        let request = format!("HEAD / HTTP/1.0\r\nHost: {ip}\r\nUser-Agent: lanping\r\n\r\n");
        timeout(wait, stream.write_all(request.as_bytes()))
            .await
            .ok()?
            .ok()?;
    }
    let mut buffer = [0u8; READ_LIMIT];
    let received = timeout(wait, stream.read(&mut buffer)).await.ok()?.ok()?;
    summarize(&buffer[..received])
}

/// Turns raw bytes into one short printable line.
pub fn summarize(bytes: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(bytes);
    let line = if text.starts_with("HTTP/") {
        http_summary(&text)
    } else {
        text.lines().next().unwrap_or_default().to_owned()
    };
    let printable: String = line
        .chars()
        .filter(|character| character.is_ascii_graphic() || *character == ' ')
        .take(MAX_BANNER_CHARS)
        .collect();
    let trimmed = printable.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The `Server` header when present, otherwise the status line.
fn http_summary(response: &str) -> String {
    let server = response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("server")
            .then(|| value.trim().to_owned())
    });
    server.unwrap_or_else(|| response.lines().next().unwrap_or_default().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn keeps_first_line_of_plain_greeting() {
        assert_eq!(
            summarize(b"SSH-2.0-OpenSSH_9.6\r\nrest").as_deref(),
            Some("SSH-2.0-OpenSSH_9.6")
        );
    }

    #[test]
    fn prefers_http_server_header() {
        let response = b"HTTP/1.1 200 OK\r\nDate: x\r\nserver: nginx/1.25\r\n\r\n";
        assert_eq!(summarize(response).as_deref(), Some("nginx/1.25"));
    }

    #[test]
    fn falls_back_to_http_status_line() {
        assert_eq!(
            summarize(b"HTTP/1.0 403 Forbidden\r\n\r\n").as_deref(),
            Some("HTTP/1.0 403 Forbidden")
        );
    }

    #[test]
    fn strips_control_characters_and_limits_length() {
        let noisy = b"\x1b[31mhello\x00 world";
        assert_eq!(summarize(noisy).as_deref(), Some("[31mhello world"));
        let long = vec![b'a'; 400];
        assert_eq!(summarize(&long).unwrap().len(), MAX_BANNER_CHARS);
        assert_eq!(summarize(b"\x00\x01"), None);
    }

    #[tokio::test]
    async fn reads_banner_from_listener() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            socket.write_all(b"220 mail.test ESMTP\r\n").await.unwrap();
        });
        let banner = grab_banner(Ipv4Addr::LOCALHOST, port, Duration::from_secs(1)).await;
        assert_eq!(banner.as_deref(), Some("220 mail.test ESMTP"));
    }

    #[tokio::test]
    async fn skips_tls_ports_and_closed_ports() {
        assert_eq!(
            grab_banner(Ipv4Addr::LOCALHOST, 443, Duration::from_millis(200)).await,
            None
        );
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        assert_eq!(
            grab_banner(Ipv4Addr::LOCALHOST, closed, Duration::from_millis(200)).await,
            None
        );
    }
}
