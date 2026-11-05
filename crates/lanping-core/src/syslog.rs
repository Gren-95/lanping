//! Receiving syslog messages from routers, NAS boxes and other devices.
//!
//! Messages in the old BSD form (RFC 3164) and the newer one (RFC 5424) are read; whatever
//! cannot be read is still kept, as its raw line. Nothing is listened for until
//! [`bind`] is called, and every message is bounded in size.

use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::AsyncReadExt;
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::{Semaphore, mpsc};

use crate::socket_budget::socket_permit;

/// The port syslog normally uses, which needs administrator rights. The default is above it.
pub const STANDARD_PORT: u16 = 514;
pub const DEFAULT_PORT: u16 = 5514;

/// Longest message kept, in bytes. A longer one is cut.
const MAX_MESSAGE_BYTES: usize = 8192;
const MAX_FACILITY: u8 = 23;
const MAX_PRIORITY: u16 = 191;
const SEVERITIES_PER_FACILITY: u16 = 8;
const MAX_CONNECTIONS: usize = 64;
const READ_CHUNK: usize = 4096;
const RFC3164_STAMP_LENGTH: usize = 15;
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const FACILITIES: [&str; 24] = [
    "kern", "user", "mail", "daemon", "auth", "syslog", "lpr", "news", "uucp", "cron", "authpriv",
    "ftp", "ntp", "audit", "alert", "clock", "local0", "local1", "local2", "local3", "local4",
    "local5", "local6", "local7",
];

/// How serious a message is. The most serious has the lowest number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Emergency,
    Alert,
    Critical,
    Error,
    Warning,
    Notice,
    Info,
    Debug,
}

impl Severity {
    pub const ALL: [Self; 8] = [
        Self::Emergency,
        Self::Alert,
        Self::Critical,
        Self::Error,
        Self::Warning,
        Self::Notice,
        Self::Info,
        Self::Debug,
    ];

    fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).copied()
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Emergency => "Emergency",
            Self::Alert => "Alert",
            Self::Critical => "Critical",
            Self::Error => "Error",
            Self::Warning => "Warning",
            Self::Notice => "Notice",
            Self::Info => "Info",
            Self::Debug => "Debug",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyslogMessage {
    /// The address the message came from.
    pub source: IpAddr,
    pub received_at_unix: u64,
    pub facility: Option<&'static str>,
    pub severity: Option<Severity>,
    /// The time the sender wrote, as it wrote it.
    pub timestamp: Option<String>,
    pub hostname: Option<String>,
    pub app: Option<String>,
    pub text: String,
    /// The whole line as received, with control characters made into spaces.
    pub raw: String,
}

/// Reads one message. Anything that is not in either form is kept as plain text.
pub fn parse(bytes: &[u8], source: IpAddr, received_at_unix: u64) -> SyslogMessage {
    let bytes = &bytes[..bytes.len().min(MAX_MESSAGE_BYTES)];
    let raw: String = String::from_utf8_lossy(bytes)
        .trim_end_matches(['\n', '\r', '\0'])
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let (priority, rest) = split_priority(&raw);
    let mut message = SyslogMessage {
        source,
        received_at_unix,
        facility: priority.and_then(|p| {
            FACILITIES
                .get(usize::from(p / SEVERITIES_PER_FACILITY as u8))
                .copied()
        }),
        severity: priority.and_then(|p| Severity::from_code(p % SEVERITIES_PER_FACILITY as u8)),
        timestamp: None,
        hostname: None,
        app: None,
        text: rest.trim().to_owned(),
        raw: raw.clone(),
    };
    if let Some(after_version) = rest.strip_prefix("1 ") {
        parse_rfc5424(after_version, &mut message);
    } else {
        parse_rfc3164(rest, &mut message);
    }
    message
}

/// The number between `<` and `>` at the start, and what follows it.
fn split_priority(text: &str) -> (Option<u8>, &str) {
    let Some(rest) = text.strip_prefix('<') else {
        return (None, text);
    };
    let Some((digits, after)) = rest.split_once('>') else {
        return (None, text);
    };
    match digits.parse::<u16>() {
        Ok(priority) if digits.len() <= 3 && priority <= MAX_PRIORITY => {
            // The facility is at most 23, so the whole number fits a byte.
            debug_assert!(priority / SEVERITIES_PER_FACILITY <= u16::from(MAX_FACILITY));
            (Some(priority as u8), after)
        }
        _ => (None, text),
    }
}

/// `Oct 11 22:14:15 host tag[123]: text`, where the host and the tag may be missing.
fn parse_rfc3164(rest: &str, message: &mut SyslogMessage) {
    let Some(stamp) = rest
        .get(..RFC3164_STAMP_LENGTH)
        .filter(|stamp| is_bsd_stamp(stamp))
    else {
        return;
    };
    message.timestamp = Some(stamp.to_owned());
    let mut remainder = rest[RFC3164_STAMP_LENGTH..].trim_start();
    // A first word that ends in a colon is the tag, so this sender left out its host name.
    if let Some((word, after)) = remainder.split_once(' ')
        && !word.ends_with(':')
    {
        message.hostname = Some(word.to_owned());
        remainder = after.trim_start();
    }
    if let Some((tag, after)) = remainder.split_once(' ')
        && let Some(tag) = tag.strip_suffix(':')
    {
        message.app = Some(tag.split('[').next().unwrap_or(tag).to_owned());
        remainder = after;
    }
    message.text = remainder.trim().to_owned();
}

/// `Mmm dd hh:mm:ss`, where a day under 10 has a space in front of it.
fn is_bsd_stamp(stamp: &str) -> bool {
    let bytes = stamp.as_bytes();
    bytes.len() == RFC3164_STAMP_LENGTH
        && MONTHS.contains(&&stamp[..3])
        && bytes[3] == b' '
        && bytes[9] == b':'
        && bytes[12] == b':'
        && bytes[10..12]
            .iter()
            .chain(&bytes[13..])
            .all(u8::is_ascii_digit)
}

/// `timestamp host app procid msgid structured-data message`, each part `-` when it is empty.
fn parse_rfc5424(rest: &str, message: &mut SyslogMessage) {
    let mut parts = rest.splitn(6, ' ');
    let (Some(timestamp), Some(host), Some(app), Some(_process), Some(_kind), Some(after)) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return;
    };
    let named = |part: &str| (part != "-").then(|| part.to_owned());
    message.timestamp = named(timestamp);
    message.hostname = named(host);
    message.app = named(app);
    let text = &after[structured_data_length(after)..];
    message.text = text.trim_start().trim_start_matches('\u{feff}').to_owned();
}

/// How many bytes of `text` are the structured data: `-`, or `[..]` groups in a row.
fn structured_data_length(text: &str) -> usize {
    if text.starts_with('-') {
        return 1;
    }
    let (mut end, mut inside, mut quoted, mut escaped) = (0, false, false, false);
    for (at, c) in text.char_indices() {
        if inside {
            match (escaped, quoted, c) {
                (true, _, _) => escaped = false,
                (false, _, '\\') => escaped = true,
                (false, _, '"') => quoted = !quoted,
                (false, false, ']') => {
                    inside = false;
                    end = at + 1;
                }
                _ => {}
            }
        } else if c == '[' {
            inside = true;
        } else {
            break;
        }
    }
    end
}

/// Cuts a stream of syslog over TCP into messages. Both ways of framing are read: one
/// message per line, and a length first (`123 <34>...`, RFC 6587).
#[derive(Default)]
struct Framer {
    buffer: Vec<u8>,
    /// The rest of a line that was too long is thrown away.
    discarding: bool,
}

/// The stream broke the framing rules and cannot be read on.
#[derive(Debug, PartialEq, Eq)]
struct BadFraming;

impl Framer {
    fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, BadFraming> {
        self.buffer.extend_from_slice(bytes);
        let mut messages = Vec::new();
        loop {
            if self.buffer.first().is_some_and(u8::is_ascii_digit) {
                let Some(space) = self.buffer.iter().position(|b| *b == b' ') else {
                    if self.buffer.len() > 8 {
                        return Err(BadFraming);
                    }
                    break;
                };
                let length = std::str::from_utf8(&self.buffer[..space])
                    .ok()
                    .and_then(|digits| digits.parse::<usize>().ok())
                    .filter(|length| *length <= MAX_MESSAGE_BYTES)
                    .ok_or(BadFraming)?;
                let end = space + 1 + length;
                if self.buffer.len() < end {
                    break;
                }
                messages.push(self.buffer[space + 1..end].to_vec());
                self.buffer.drain(..end);
            } else if let Some(newline) = self.buffer.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=newline).collect();
                if std::mem::take(&mut self.discarding) {
                    continue;
                }
                if line.len() > 1 {
                    messages.push(line[..line.len() - 1].to_vec());
                }
            } else {
                if self.buffer.len() > MAX_MESSAGE_BYTES {
                    self.buffer.clear();
                    self.discarding = true;
                }
                break;
            }
        }
        Ok(messages)
    }
}

/// The sockets of a listener that has been started.
pub struct Listening {
    pub port: u16,
    udp: UdpSocket,
    tcp: Option<TcpListener>,
}

/// Starts listening on `port`, over UDP and, when asked, TCP too. Port 0 picks a free one.
pub async fn bind(port: u16, with_tcp: bool) -> io::Result<Listening> {
    let udp = UdpSocket::bind(("0.0.0.0", port)).await?;
    let port = udp.local_addr()?.port();
    let tcp = if with_tcp {
        Some(TcpListener::bind(("0.0.0.0", port)).await?)
    } else {
        None
    };
    Ok(Listening { port, udp, tcp })
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

impl Listening {
    /// Reports each message on `sink` until the future is dropped. A message that finds the
    /// sink full is dropped, so a flood cannot grow memory without bound.
    pub async fn run(self, sink: mpsc::Sender<SyslogMessage>) {
        let udp = Self::receive_datagrams(self.udp, sink.clone());
        match self.tcp {
            Some(tcp) => {
                tokio::join!(udp, Self::accept_streams(tcp, sink));
            }
            None => udp.await,
        }
    }

    async fn receive_datagrams(socket: UdpSocket, sink: mpsc::Sender<SyslogMessage>) {
        let mut buffer = vec![0; MAX_MESSAGE_BYTES + 1];
        loop {
            let Ok((length, from)) = socket.recv_from(&mut buffer).await else {
                continue;
            };
            let _ = sink.try_send(parse(&buffer[..length], from.ip(), unix_now()));
        }
    }

    async fn accept_streams(listener: TcpListener, sink: mpsc::Sender<SyslogMessage>) {
        let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
        loop {
            let Ok((stream, peer)) = listener.accept().await else {
                continue;
            };
            let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                continue;
            };
            let sink = sink.clone();
            tokio::spawn(async move {
                let _slot = slot;
                let _permit = socket_permit().await;
                read_stream(stream, peer, sink).await;
            });
        }
    }
}

async fn read_stream(mut stream: TcpStream, peer: SocketAddr, sink: mpsc::Sender<SyslogMessage>) {
    let mut framer = Framer::default();
    let mut chunk = [0u8; READ_CHUNK];
    loop {
        // A listener that was stopped closes the sink; its connections end with it.
        let length = tokio::select! {
            () = sink.closed() => return,
            read = stream.read(&mut chunk) => match read {
                Ok(0) | Err(_) => return,
                Ok(length) => length,
            },
        };
        let Ok(messages) = framer.push(&chunk[..length]) else {
            return;
        };
        for message in messages {
            let _ = sink.try_send(parse(&message, peer.ip(), unix_now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::time::Duration;
    use tokio::io::AsyncWriteExt;

    const SOURCE: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 168, 0, 1));

    fn read(text: &str) -> SyslogMessage {
        parse(text.as_bytes(), SOURCE, 1000)
    }

    #[test]
    fn the_old_bsd_form_gives_its_host_tag_and_text() {
        let message = read("<34>Oct 11 22:14:15 mymachine su[1234]: 'su root' failed for lonvick");
        assert_eq!(message.facility, Some("auth"));
        assert_eq!(message.severity, Some(Severity::Critical));
        assert_eq!(message.timestamp.as_deref(), Some("Oct 11 22:14:15"));
        assert_eq!(message.hostname.as_deref(), Some("mymachine"));
        assert_eq!(message.app.as_deref(), Some("su"));
        assert_eq!(message.text, "'su root' failed for lonvick");
    }

    #[test]
    fn a_sender_that_leaves_out_its_host_name_is_still_read() {
        let message = read("<30>Nov  5 22:09:01 dnsmasq-dhcp[123]: DHCPACK(br0) 192.168.0.9");
        assert_eq!(message.hostname, None);
        assert_eq!(message.app.as_deref(), Some("dnsmasq-dhcp"));
        assert_eq!(message.text, "DHCPACK(br0) 192.168.0.9");
        assert_eq!(message.severity, Some(Severity::Info));
        assert_eq!(message.facility, Some("daemon"));
    }

    #[test]
    fn a_message_with_no_tag_keeps_all_its_words() {
        let message = read("<13>Feb  5 17:32:18 10.0.0.99 Use the BFG!");
        assert_eq!(message.hostname.as_deref(), Some("10.0.0.99"));
        assert_eq!(message.app, None);
        assert_eq!(message.text, "Use the BFG!");
    }

    #[test]
    fn the_newer_form_gives_its_fields_and_skips_the_structured_data() {
        let message = read(
            "<165>1 2003-10-11T22:14:15.003Z mymachine.example.com evntslog - ID47 \
             [exampleSDID@32473 iut=\"3\" note=\"a ] in a value\"][other x=\"1\"] \u{feff}An application event",
        );
        assert_eq!(message.facility, Some("local4"));
        assert_eq!(message.severity, Some(Severity::Notice));
        assert_eq!(
            message.timestamp.as_deref(),
            Some("2003-10-11T22:14:15.003Z")
        );
        assert_eq!(message.hostname.as_deref(), Some("mymachine.example.com"));
        assert_eq!(message.app.as_deref(), Some("evntslog"));
        assert_eq!(message.text, "An application event");
    }

    #[test]
    fn dashes_in_the_newer_form_mean_nothing_was_given() {
        let message = read("<34>1 - - - - - - plain text");
        assert_eq!(
            (message.timestamp, message.hostname, message.app),
            (None, None, None)
        );
        assert_eq!(message.text, "plain text");
    }

    #[test]
    fn a_message_that_is_in_neither_form_is_kept_as_it_came() {
        let message = read("hello from a device with no header");
        assert_eq!(message.severity, None);
        assert_eq!(message.text, "hello from a device with no header");
        assert_eq!(message.raw, "hello from a device with no header");
        let broken = read("<999>Oct 11 22:14:15 x y: z");
        assert_eq!(broken.severity, None);
        assert_eq!(read("<34").text, "<34");
        assert_eq!(read("").text, "");
    }

    #[test]
    fn control_characters_and_huge_messages_are_made_safe() {
        let message = read("<14>Oct 11 22:14:15 h a: bell\u{7} and\nline\r\n");
        assert_eq!(message.text, "bell  and line");
        let huge = parse(&vec![b'x'; 100_000], SOURCE, 0);
        assert_eq!(huge.raw.len(), MAX_MESSAGE_BYTES);
    }

    #[test]
    fn the_severity_numbers_run_from_most_to_least_serious() {
        assert!(Severity::Emergency < Severity::Warning && Severity::Warning < Severity::Debug);
        assert_eq!(read("<0>x").severity, Some(Severity::Emergency));
        assert_eq!(read("<191>x").severity, Some(Severity::Debug));
        assert_eq!(read("<191>x").facility, Some("local7"));
    }

    #[test]
    fn lines_over_tcp_are_cut_apart_and_a_partial_one_waits() {
        let mut framer = Framer::default();
        assert_eq!(
            framer.push(b"<14>one\n<14>tw").unwrap(),
            [b"<14>one".to_vec()]
        );
        assert_eq!(
            framer.push(b"o\n\n<14>three\n").unwrap(),
            [b"<14>two".to_vec(), b"<14>three".to_vec()]
        );
    }

    #[test]
    fn counted_frames_over_tcp_are_read_even_when_split() {
        let mut framer = Framer::default();
        assert!(framer.push(b"7 <14>on").unwrap().is_empty());
        assert_eq!(
            framer.push(b"e5 <14>x").unwrap(),
            [b"<14>one".to_vec(), b"<14>x".to_vec()]
        );
    }

    #[test]
    fn a_line_that_is_too_long_is_dropped_whole() {
        let mut framer = Framer::default();
        assert!(
            framer
                .push(&vec![b'a'; MAX_MESSAGE_BYTES + 10])
                .unwrap()
                .is_empty()
        );
        assert!(framer.push(b"more of it\n").unwrap().is_empty());
        assert_eq!(framer.push(b"<14>next\n").unwrap(), [b"<14>next".to_vec()]);
    }

    #[test]
    fn a_stream_that_breaks_the_framing_is_refused() {
        assert_eq!(Framer::default().push(b"99999999 x"), Err(BadFraming));
        assert_eq!(Framer::default().push(b"1234567890123"), Err(BadFraming));
    }

    #[tokio::test]
    async fn datagrams_are_received_and_reported_with_their_source() {
        let listening = bind(0, false).await.unwrap();
        let port = listening.port;
        let (sink, mut messages) = mpsc::channel(8);
        let task = tokio::spawn(listening.run(sink));
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        sender
            .send_to(
                b"<11>Nov  5 22:09:01 router app: link down\n",
                ("127.0.0.1", port),
            )
            .await
            .unwrap();
        let message = tokio::time::timeout(Duration::from_secs(2), messages.recv())
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert_eq!(message.source, IpAddr::V4(Ipv4Addr::LOCALHOST));
        assert_eq!(
            (message.hostname.as_deref(), message.text.as_str()),
            (Some("router"), "link down")
        );
        assert_eq!(message.severity, Some(Severity::Error));
    }

    #[tokio::test]
    async fn a_busy_sink_drops_messages_instead_of_growing() {
        let listening = bind(0, false).await.unwrap();
        let port = listening.port;
        let (sink, mut messages) = mpsc::channel(2);
        let task = tokio::spawn(listening.run(sink));
        let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        for _ in 0..50 {
            sender
                .send_to(b"<14>flood", ("127.0.0.1", port))
                .await
                .unwrap();
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        task.abort();
        let mut kept = 0;
        while messages.try_recv().is_ok() {
            kept += 1;
        }
        assert!(kept <= 2, "{kept}");
    }

    #[tokio::test]
    async fn lines_sent_over_tcp_are_received() {
        let tcp = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = tcp.local_addr().unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let listening = Listening {
            port: address.port(),
            udp,
            tcp: Some(tcp),
        };
        let (sink, mut messages) = mpsc::channel(8);
        let task = tokio::spawn(listening.run(sink));
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"<14>Nov  5 22:09:01 nas app: one\n<14>Nov  5 22:09:02 nas app: two\n")
            .await
            .unwrap();
        let mut texts = Vec::new();
        for _ in 0..2 {
            let message = tokio::time::timeout(Duration::from_secs(2), messages.recv())
                .await
                .unwrap()
                .unwrap();
            texts.push(message.text);
        }
        task.abort();
        assert_eq!(texts, ["one", "two"]);
    }
}
