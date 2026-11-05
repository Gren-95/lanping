//! How far off this computer's clock is, by asking a time server over SNTP (RFC 4330).
//!
//! One 48-byte request goes out and one comes back. Four times give the answer: when the
//! request left (T1), when the server got it (T2), when it answered (T3) and when the
//! answer arrived (T4). Nothing on this computer is ever changed.

use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::net::{UdpSocket, lookup_host};
use tokio::time::timeout;

use crate::socket_budget::socket_permit;

pub const NTP_PORT: u16 = 123;
/// Servers the picker offers.
pub const WELL_KNOWN_SERVERS: [&str; 4] = [
    "pool.ntp.org",
    "time.cloudflare.com",
    "time.google.com",
    "time.windows.com",
];

const PACKET_LENGTH: usize = 48;
/// Version 4, client mode, no leap warning.
const CLIENT_HEADER: u8 = 0b00_100_011;
const MODE_MASK: u8 = 0b111;
const MODE_SERVER: u8 = 4;
const VERSION_SHIFT: u8 = 3;
const VERSION_MASK: u8 = 0b111;
const LEAP_SHIFT: u8 = 6;
const LEAP_ALARM: u8 = 3;
const MAX_STRATUM: u8 = 15;
const STRATUM_BYTE: usize = 1;
const KISS_CODE: std::ops::Range<usize> = 12..16;
const ORIGINATE: std::ops::Range<usize> = 24..32;
const RECEIVE: std::ops::Range<usize> = 32..40;
const TRANSMIT: std::ops::Range<usize> = 40..48;
/// Seconds between 1900, where NTP starts counting, and 1970.
const NTP_TO_UNIX_SECONDS: u64 = 2_208_988_800;
const FRACTION_UNITS: f64 = 4_294_967_296.0;
const NANOS_PER_SECOND: u64 = 1_000_000_000;
const ANSWER_WAIT: Duration = Duration::from_secs(3);
const ATTEMPTS: usize = 2;
const RECEIVE_BUFFER: usize = 512;

/// An NTP time: 32 bits of seconds since 1900 and 32 bits of fraction. It wraps in 2036, so
/// only differences are used, and those stay right across the wrap.
type Timestamp = u64;

/// What a time server said about this computer's clock.
#[derive(Debug, Clone, PartialEq)]
pub struct NtpReport {
    pub server: String,
    pub address: IpAddr,
    /// 1 is a server with its own clock; higher numbers are further from one.
    pub stratum: u8,
    /// How far this computer's clock is ahead of the server's. Negative means behind.
    pub clock_ahead_seconds: f64,
    /// The time the request and the answer spent travelling.
    pub round_trip_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NtpError {
    BadServer,
    Lookup(String),
    NoAnswer,
    /// The server told this client to stay away. Carries its four-letter code.
    Refused(String),
    /// The server's own clock is not set right, so its time cannot be trusted.
    Unsynchronised,
    /// A reply that is not an answer to this request.
    BadReply,
    Failed(String),
}

impl fmt::Display for NtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadServer => f.write_str("Enter a server name or address, such as pool.ntp.org"),
            Self::Lookup(error) => write!(f, "Could not find that server: {error}"),
            Self::NoAnswer => f.write_str(
                "The server did not answer. Check the name and that UDP port 123 is not blocked.",
            ),
            Self::Refused(code) => write!(f, "The server refused the request ({code})"),
            Self::Unsynchronised => {
                f.write_str("The server's own clock is not set, so it cannot be used")
            }
            Self::BadReply => f.write_str("The server sent something that is not a time answer"),
            Self::Failed(error) => write!(f, "Could not ask the server: {error}"),
        }
    }
}

/// `pool.ntp.org`, `pool.ntp.org:123`, `192.168.0.1` or `[fe80::1]:123`.
pub fn parse_server(text: &str) -> Result<(String, u16), NtpError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(NtpError::BadServer);
    }
    if let Ok(address) = text.parse::<SocketAddr>() {
        return Ok((address.ip().to_string(), address.port()));
    }
    if text.parse::<IpAddr>().is_ok() {
        return Ok((text.to_owned(), NTP_PORT));
    }
    match text.split_once(':') {
        Some((host, port)) if !host.is_empty() && !port.contains(':') => port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .map(|port| (host.to_owned(), port))
            .ok_or(NtpError::BadServer),
        Some(_) => Err(NtpError::BadServer),
        None => Ok((text.to_owned(), NTP_PORT)),
    }
}

/// Asks `server` (see [`parse_server`]) for the time and compares it with this clock.
pub async fn check(server: &str) -> Result<NtpReport, NtpError> {
    let (host, port) = parse_server(server)?;
    let address = lookup_host((host.as_str(), port))
        .await
        .map_err(|error| NtpError::Lookup(error.to_string()))?
        .next()
        .ok_or_else(|| NtpError::Lookup("no address".to_owned()))?;
    let _permit = socket_permit().await;
    let bind = if address.is_ipv4() {
        "0.0.0.0:0"
    } else {
        "[::]:0"
    };
    let socket = UdpSocket::bind(bind)
        .await
        .map_err(|error| NtpError::Failed(error.to_string()))?;
    socket
        .connect(address)
        .await
        .map_err(|error| NtpError::Failed(error.to_string()))?;
    for _ in 0..ATTEMPTS {
        match ask_once(&socket).await {
            Err(NtpError::NoAnswer) => continue,
            Ok(answer) => {
                return Ok(NtpReport {
                    server: host,
                    address: address.ip(),
                    stratum: answer.stratum,
                    clock_ahead_seconds: -answer.offset_seconds,
                    round_trip_seconds: answer.delay_seconds,
                });
            }
            Err(error) => return Err(error),
        }
    }
    Err(NtpError::NoAnswer)
}

struct Answer {
    stratum: u8,
    /// How far the server is ahead of this clock.
    offset_seconds: f64,
    delay_seconds: f64,
}

/// One request and its answer, ignoring anything that is not an answer to it.
async fn ask_once(socket: &UdpSocket) -> Result<Answer, NtpError> {
    let sent_at = Instant::now();
    let t1 = now_timestamp();
    socket
        .send(&request(t1))
        .await
        .map_err(|error| NtpError::Failed(error.to_string()))?;
    let deadline = sent_at + ANSWER_WAIT;
    let mut buffer = [0u8; RECEIVE_BUFFER];
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        let length = match timeout(wait, socket.recv(&mut buffer)).await {
            Ok(Ok(length)) => length,
            Ok(Err(error)) => return Err(NtpError::Failed(error.to_string())),
            Err(_) => return Err(NtpError::NoAnswer),
        };
        // The arrival time comes from the monotonic clock, so a clock that is being
        // corrected during the exchange does not spoil the round trip.
        let t4 = t1.wrapping_add(duration_to_timestamp(sent_at.elapsed()));
        match parse_reply(&buffer[..length], t1) {
            Ok(reply) => {
                let (offset_seconds, delay_seconds) = offset_and_delay(t1, reply.t2, reply.t3, t4);
                return Ok(Answer {
                    stratum: reply.stratum,
                    offset_seconds,
                    delay_seconds,
                });
            }
            Err(NtpError::BadReply) => continue,
            Err(error) => return Err(error),
        }
    }
}

fn request(transmit: Timestamp) -> [u8; PACKET_LENGTH] {
    let mut packet = [0; PACKET_LENGTH];
    packet[0] = CLIENT_HEADER;
    packet[TRANSMIT].copy_from_slice(&transmit.to_be_bytes());
    packet
}

struct Reply {
    stratum: u8,
    t2: Timestamp,
    t3: Timestamp,
}

/// Reads an answer to the request that left at `originate`. A reply that does not echo it
/// is not an answer to this request, and is [`NtpError::BadReply`].
fn parse_reply(packet: &[u8], originate: Timestamp) -> Result<Reply, NtpError> {
    if packet.len() < PACKET_LENGTH {
        return Err(NtpError::BadReply);
    }
    let header = packet[0];
    let version = (header >> VERSION_SHIFT) & VERSION_MASK;
    if header & MODE_MASK != MODE_SERVER
        || version == 0
        || timestamp_at(packet, ORIGINATE) != originate
    {
        return Err(NtpError::BadReply);
    }
    let stratum = packet[STRATUM_BYTE];
    if stratum == 0 {
        let code = String::from_utf8_lossy(&packet[KISS_CODE])
            .trim_matches('\0')
            .chars()
            .filter(char::is_ascii_graphic)
            .collect();
        return Err(NtpError::Refused(code));
    }
    if header >> LEAP_SHIFT == LEAP_ALARM || stratum > MAX_STRATUM {
        return Err(NtpError::Unsynchronised);
    }
    let (t2, t3) = (
        timestamp_at(packet, RECEIVE),
        timestamp_at(packet, TRANSMIT),
    );
    if t3 == 0 {
        return Err(NtpError::BadReply);
    }
    Ok(Reply { stratum, t2, t3 })
}

fn timestamp_at(packet: &[u8], range: std::ops::Range<usize>) -> Timestamp {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&packet[range]);
    Timestamp::from_be_bytes(bytes)
}

/// How far the server is ahead of this clock, and the round trip, in seconds.
fn offset_and_delay(t1: Timestamp, t2: Timestamp, t3: Timestamp, t4: Timestamp) -> (f64, f64) {
    let outbound = signed_difference(t2, t1);
    let inbound = signed_difference(t3, t4);
    let offset = (i128::from(outbound) + i128::from(inbound)) as f64 / 2.0;
    let delay = signed_difference(t4, t1) - signed_difference(t3, t2);
    (offset / FRACTION_UNITS, delay as f64 / FRACTION_UNITS)
}

/// `later - earlier`, right across the wrap of the seconds counter.
fn signed_difference(later: Timestamp, earlier: Timestamp) -> i64 {
    later.wrapping_sub(earlier) as i64
}

fn now_timestamp() -> Timestamp {
    let since_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = since_unix.as_secs().wrapping_add(NTP_TO_UNIX_SECONDS) & u64::from(u32::MAX);
    (seconds << 32) | fraction_of(since_unix.subsec_nanos())
}

fn duration_to_timestamp(duration: Duration) -> Timestamp {
    (duration.as_secs() << 32).wrapping_add(fraction_of(duration.subsec_nanos()))
}

fn fraction_of(nanos: u32) -> u64 {
    (u64::from(nanos) << 32) / NANOS_PER_SECOND
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: u64, fraction: u64) -> Timestamp {
        (seconds << 32) | fraction
    }

    fn answer(originate: Timestamp, t2: Timestamp, t3: Timestamp) -> [u8; PACKET_LENGTH] {
        let mut packet = [0; PACKET_LENGTH];
        packet[0] = 0b00_100_100;
        packet[STRATUM_BYTE] = 2;
        packet[ORIGINATE].copy_from_slice(&originate.to_be_bytes());
        packet[RECEIVE].copy_from_slice(&t2.to_be_bytes());
        packet[TRANSMIT].copy_from_slice(&t3.to_be_bytes());
        packet
    }

    #[test]
    fn a_request_is_a_client_packet_that_carries_the_time_it_left() {
        let packet = request(at(5, 6));
        assert_eq!(packet.len(), 48);
        assert_eq!(packet[0] & MODE_MASK, 3);
        assert_eq!(timestamp_at(&packet, TRANSMIT), at(5, 6));
    }

    #[test]
    fn a_server_ahead_by_five_seconds_over_a_quarter_second_trip_is_measured() {
        // The request leaves at 100 and arrives 0.125 s later by the server's clock, which
        // is 5 s ahead. It answers at once and the answer takes 0.125 s back.
        let half = 1 << 31;
        let t1 = at(100, 0);
        let t2 = at(105, half / 4);
        let t3 = t2;
        let t4 = at(100, half / 2);
        let (offset, delay) = offset_and_delay(t1, t2, t3, t4);
        assert!((offset - 5.0).abs() < 1e-6, "{offset}");
        assert!((delay - 0.25).abs() < 1e-6, "{delay}");
    }

    #[test]
    fn the_time_the_server_takes_to_answer_is_not_counted_as_travel() {
        let t1 = at(10, 0);
        let t2 = at(10, 0);
        let t3 = at(12, 0);
        let t4 = at(12, 0);
        let (offset, delay) = offset_and_delay(t1, t2, t3, t4);
        assert_eq!((offset, delay), (0.0, 0.0));
    }

    #[test]
    fn differences_stay_right_across_the_wrap_of_2036() {
        let before_wrap = at(u64::from(u32::MAX), 0);
        let after_wrap = at(1, 0);
        assert_eq!(signed_difference(after_wrap, before_wrap), 2 << 32);
        assert_eq!(signed_difference(before_wrap, after_wrap), -(2 << 32));
    }

    #[test]
    fn the_clock_reads_as_a_time_after_2020() {
        let seconds = now_timestamp() >> 32;
        assert!(seconds > 3_786_825_600, "{seconds}");
    }

    #[test]
    fn an_answer_is_read_when_it_echoes_the_request() {
        let sent = at(7, 9);
        let reply = parse_reply(&answer(sent, at(8, 0), at(8, 1)), sent).unwrap();
        assert_eq!((reply.stratum, reply.t2, reply.t3), (2, at(8, 0), at(8, 1)));
    }

    #[test]
    fn a_reply_that_does_not_echo_the_request_is_not_an_answer() {
        let packet = answer(at(1, 1), at(8, 0), at(8, 1));
        assert_eq!(
            parse_reply(&packet, at(7, 9)).err(),
            Some(NtpError::BadReply)
        );
        assert_eq!(
            parse_reply(&packet[..20], at(1, 1)).err(),
            Some(NtpError::BadReply)
        );
        let mut client_packet = packet;
        client_packet[0] = CLIENT_HEADER;
        assert_eq!(
            parse_reply(&client_packet, at(1, 1)).err(),
            Some(NtpError::BadReply)
        );
        let unset = answer(at(1, 1), at(8, 0), 0);
        assert_eq!(
            parse_reply(&unset, at(1, 1)).err(),
            Some(NtpError::BadReply)
        );
    }

    #[test]
    fn a_server_that_says_go_away_is_reported_with_its_code() {
        let mut packet = answer(at(1, 1), at(8, 0), at(8, 1));
        packet[STRATUM_BYTE] = 0;
        packet[KISS_CODE].copy_from_slice(b"RATE");
        assert_eq!(
            parse_reply(&packet, at(1, 1)).err(),
            Some(NtpError::Refused("RATE".to_owned()))
        );
    }

    #[test]
    fn a_server_with_no_clock_of_its_own_is_not_used() {
        let mut alarm = answer(at(1, 1), at(8, 0), at(8, 1));
        alarm[0] |= LEAP_ALARM << LEAP_SHIFT;
        assert_eq!(
            parse_reply(&alarm, at(1, 1)).err(),
            Some(NtpError::Unsynchronised)
        );
        let mut far = answer(at(1, 1), at(8, 0), at(8, 1));
        far[STRATUM_BYTE] = 16;
        assert_eq!(
            parse_reply(&far, at(1, 1)).err(),
            Some(NtpError::Unsynchronised)
        );
    }

    #[test]
    fn servers_are_read_with_and_without_a_port() {
        assert_eq!(
            parse_server(" pool.ntp.org "),
            Ok(("pool.ntp.org".to_owned(), 123))
        );
        assert_eq!(
            parse_server("10.0.0.1:1230"),
            Ok(("10.0.0.1".to_owned(), 1230))
        );
        assert_eq!(
            parse_server("192.168.0.1"),
            Ok(("192.168.0.1".to_owned(), 123))
        );
        assert_eq!(
            parse_server("[fe80::1]:124"),
            Ok(("fe80::1".to_owned(), 124))
        );
        assert_eq!(parse_server("fe80::1"), Ok(("fe80::1".to_owned(), 123)));
        for bad in ["", "  ", ":123", "host:0", "host:x", "host:70000", "a:1:2"] {
            assert_eq!(parse_server(bad), Err(NtpError::BadServer), "{bad:?}");
        }
    }

    #[tokio::test]
    async fn a_server_on_this_computer_is_asked_and_its_offset_is_found() {
        let server = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = server.local_addr().unwrap().port();
        tokio::spawn(async move {
            let mut buffer = [0u8; RECEIVE_BUFFER];
            let (length, peer) = server.recv_from(&mut buffer).await.unwrap();
            assert_eq!(length, PACKET_LENGTH);
            let originate = timestamp_at(&buffer, TRANSMIT);
            // This server's clock is 30 seconds ahead.
            let ahead = originate.wrapping_add(30 << 32);
            server
                .send_to(&answer(originate, ahead, ahead), peer)
                .await
                .unwrap();
        });
        let report = check(&format!("127.0.0.1:{port}")).await.unwrap();
        assert_eq!(report.stratum, 2);
        assert!(
            (report.clock_ahead_seconds + 30.0).abs() < 0.5,
            "{report:?}"
        );
        assert!(report.round_trip_seconds < 1.0);
    }
}
