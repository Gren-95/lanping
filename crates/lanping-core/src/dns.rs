//! DNS lookups straight to a chosen resolver over UDP (and TCP when an answer is cut
//! short), and a timing comparison of resolvers. No system resolver tools are needed.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

use crate::mdns::{read_name, read_u16};
use crate::socket_budget::socket_permit;

const DNS_PORT: u16 = 53;
const QUERY_WAIT: Duration = Duration::from_secs(3);
/// Size of answer this client says it accepts over UDP (EDNS0).
const EDNS_PAYLOAD: u16 = 1232;
const MAX_NAME_LENGTH: usize = 253;
const MAX_LABEL_LENGTH: usize = 63;
const HEADER_LENGTH: usize = 12;
const CLASS_IN: u16 = 1;
const TYPE_OPT: u16 = 41;
const FLAG_RESPONSE: u16 = 0x8000;
const FLAG_TRUNCATED: u16 = 0x0200;
const FLAG_RECURSION_DESIRED: u16 = 0x0100;
const RCODE_MASK: u16 = 0x000F;
const RCODE_NAME_ERROR: u16 = 3;
const RECEIVE_BUFFER: usize = 4096;
/// Queries sent to each resolver when comparing them.
pub const BENCHMARK_ROUNDS: usize = 3;

/// Public resolvers offered next to the ones the system uses.
pub const PUBLIC_RESOLVERS: [(&str, IpAddr); 3] = [
    ("Cloudflare", IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
    ("Google", IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))),
    ("Quad9", IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordType {
    A,
    Aaaa,
    Mx,
    Txt,
    Ns,
    Ptr,
    Cname,
}

impl RecordType {
    pub const ALL: [Self; 7] = [
        Self::A,
        Self::Aaaa,
        Self::Mx,
        Self::Txt,
        Self::Ns,
        Self::Ptr,
        Self::Cname,
    ];

    pub fn code(self) -> u16 {
        match self {
            Self::A => 1,
            Self::Ns => 2,
            Self::Cname => 5,
            Self::Ptr => 12,
            Self::Mx => 15,
            Self::Txt => 16,
            Self::Aaaa => 28,
        }
    }

    fn from_code(code: u16) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.code() == code)
    }
}

impl fmt::Display for RecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Mx => "MX",
            Self::Txt => "TXT",
            Self::Ns => "NS",
            Self::Ptr => "PTR",
            Self::Cname => "CNAME",
        })
    }
}

/// One record of an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsRecord {
    pub type_code: u16,
    /// Seconds the answer may be cached.
    pub ttl: u32,
    /// The value in text form: an address, a host name, `10 mail.example.com`, or text.
    pub data: String,
}

impl DnsRecord {
    pub fn type_name(&self) -> String {
        RecordType::from_code(self.type_code).map_or_else(
            || format!("TYPE{}", self.type_code),
            |kind| kind.to_string(),
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Answer {
    pub records: Vec<DnsRecord>,
    pub server: IpAddr,
    pub elapsed: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DnsError {
    InvalidName,
    Timeout,
    /// The name does not exist (NXDOMAIN).
    NotFound,
    /// The resolver answered with this error code.
    ServerFailure(u16),
    Malformed,
    Network(String),
}

impl fmt::Display for DnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName => f.write_str("Enter a host name or an IP address"),
            Self::Timeout => f.write_str("The resolver did not answer in time"),
            Self::NotFound => f.write_str("The name does not exist"),
            Self::ServerFailure(code) => write!(f, "The resolver reported an error (code {code})"),
            Self::Malformed => f.write_str("The resolver sent an answer that could not be read"),
            Self::Network(reason) => write!(f, "Could not reach the resolver: {reason}"),
        }
    }
}

/// Resolvers the system is set to use, in order. Empty where they cannot be read.
pub fn system_resolvers() -> Vec<IpAddr> {
    #[cfg(unix)]
    {
        std::fs::read_to_string("/etc/resolv.conf")
            .map(|text| parse_resolv_conf(&text))
            .unwrap_or_default()
    }
    #[cfg(not(unix))]
    {
        Vec::new()
    }
}

/// The `nameserver` lines of a `resolv.conf` file.
pub fn parse_resolv_conf(text: &str) -> Vec<IpAddr> {
    text.lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            if words.next()? != "nameserver" {
                return None;
            }
            // A link-local address may carry a scope, as in `fe80::1%eth0`.
            words.next()?.split('%').next()?.parse().ok()
        })
        .collect()
}

/// `4.3.2.1.in-addr.arpa` or the nibble form under `ip6.arpa`.
pub fn reverse_name(address: IpAddr) -> String {
    match address {
        IpAddr::V4(address) => {
            let [a, b, c, d] = address.octets();
            format!("{d}.{c}.{b}.{a}.in-addr.arpa")
        }
        IpAddr::V6(address) => {
            let nibbles: Vec<String> = address
                .octets()
                .iter()
                .rev()
                .flat_map(|byte| [byte & 0x0F, byte >> 4])
                .map(|nibble| format!("{nibble:x}"))
                .collect();
            format!("{}.ip6.arpa", nibbles.join("."))
        }
    }
}

/// The name to ask for: the reverse name when a PTR is asked for an address, otherwise the
/// typed name without a trailing dot.
pub(crate) fn question_name(input: &str, record_type: RecordType) -> Result<String, DnsError> {
    let input = input.trim();
    if record_type == RecordType::Ptr
        && let Ok(address) = input.parse::<IpAddr>()
    {
        return Ok(reverse_name(address));
    }
    let name = input.strip_suffix('.').unwrap_or(input);
    let valid_label = |label: &str| {
        !label.is_empty()
            && label.len() <= MAX_LABEL_LENGTH
            && label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    };
    let valid = !name.is_empty()
        && name.len() <= MAX_NAME_LENGTH
        && !name.starts_with('-')
        && name.split('.').all(valid_label);
    if valid {
        Ok(name.to_owned())
    } else {
        Err(DnsError::InvalidName)
    }
}

fn next_query_id() -> u16 {
    static COUNTER: AtomicU16 = AtomicU16::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    (nanos as u16) ^ COUNTER.fetch_add(0x9E37, Ordering::Relaxed)
}

fn query_packet(id: u16, name: &str, record_type: RecordType) -> Vec<u8> {
    let mut packet = Vec::with_capacity(HEADER_LENGTH + name.len() + 20);
    packet.extend(id.to_be_bytes());
    packet.extend(FLAG_RECURSION_DESIRED.to_be_bytes());
    // One question, no answers, one additional record (the EDNS0 option).
    packet.extend([0, 1, 0, 0, 0, 0, 0, 1]);
    for label in name.split('.') {
        packet.push(label.len() as u8);
        packet.extend(label.as_bytes());
    }
    packet.push(0);
    packet.extend(record_type.code().to_be_bytes());
    packet.extend(CLASS_IN.to_be_bytes());
    // EDNS0: root name, type OPT, the UDP size in the class field, no data.
    packet.push(0);
    packet.extend(TYPE_OPT.to_be_bytes());
    packet.extend(EDNS_PAYLOAD.to_be_bytes());
    packet.extend([0, 0, 0, 0, 0, 0]);
    packet
}

/// Asks `server` for `record_type` records of `input`. An address typed for a PTR
/// lookup is turned into its reverse name first.
pub async fn lookup(
    server: IpAddr,
    input: &str,
    record_type: RecordType,
) -> Result<Answer, DnsError> {
    let name = question_name(input, record_type)?;
    let id = next_query_id();
    let packet = query_packet(id, &name, record_type);
    let address = SocketAddr::new(server, DNS_PORT);
    let started = Instant::now();
    let mut response = parse_response(&exchange_udp(address, &packet).await?, id)?;
    if response.truncated {
        response = parse_response(&exchange_tcp(address, &packet).await?, id)?;
    }
    Ok(Answer {
        records: response.records,
        server,
        elapsed: started.elapsed(),
    })
}

fn network_error(error: std::io::Error) -> DnsError {
    DnsError::Network(error.to_string())
}

async fn exchange_udp(server: SocketAddr, packet: &[u8]) -> Result<Vec<u8>, DnsError> {
    let _permit = socket_permit().await;
    let local = if server.is_ipv4() {
        SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0))
    } else {
        SocketAddr::from((Ipv6Addr::UNSPECIFIED, 0))
    };
    let socket = UdpSocket::bind(local).await.map_err(network_error)?;
    socket.connect(server).await.map_err(network_error)?;
    socket.send(packet).await.map_err(network_error)?;
    let mut buffer = vec![0; RECEIVE_BUFFER];
    let length = timeout(QUERY_WAIT, socket.recv(&mut buffer))
        .await
        .map_err(|_| DnsError::Timeout)?
        .map_err(network_error)?;
    buffer.truncate(length);
    Ok(buffer)
}

async fn exchange_tcp(server: SocketAddr, packet: &[u8]) -> Result<Vec<u8>, DnsError> {
    let _permit = socket_permit().await;
    let exchange = async {
        let mut stream = TcpStream::connect(server).await?;
        stream
            .write_all(&(packet.len() as u16).to_be_bytes())
            .await?;
        stream.write_all(packet).await?;
        let mut length = [0; 2];
        stream.read_exact(&mut length).await?;
        let mut response = vec![0; usize::from(u16::from_be_bytes(length))];
        stream.read_exact(&mut response).await?;
        Ok(response)
    };
    timeout(QUERY_WAIT, exchange)
        .await
        .map_err(|_| DnsError::Timeout)?
        .map_err(network_error)
}

struct Response {
    truncated: bool,
    records: Vec<DnsRecord>,
}

fn parse_response(packet: &[u8], expected_id: u16) -> Result<Response, DnsError> {
    let malformed = || DnsError::Malformed;
    if packet.len() < HEADER_LENGTH || read_u16(packet, 0) != Some(expected_id) {
        return Err(malformed());
    }
    let flags = read_u16(packet, 2).ok_or_else(malformed)?;
    if flags & FLAG_RESPONSE == 0 {
        return Err(malformed());
    }
    if flags & FLAG_TRUNCATED != 0 {
        return Ok(Response {
            truncated: true,
            records: Vec::new(),
        });
    }
    match flags & RCODE_MASK {
        0 => {}
        RCODE_NAME_ERROR => return Err(DnsError::NotFound),
        code => return Err(DnsError::ServerFailure(code)),
    }
    let questions = read_u16(packet, 4).ok_or_else(malformed)?;
    let answers = read_u16(packet, 6).ok_or_else(malformed)?;
    let mut position = HEADER_LENGTH;
    for _ in 0..questions {
        let (_, after_name) = read_name(packet, position).ok_or_else(malformed)?;
        position = after_name + 4;
    }
    let mut records = Vec::new();
    for _ in 0..answers {
        let (record, after) = read_record(packet, position).ok_or_else(malformed)?;
        position = after;
        records.push(record);
    }
    Ok(Response {
        truncated: false,
        records,
    })
}

/// Reads one resource record. Returns it and the offset just after it.
fn read_record(packet: &[u8], start: usize) -> Option<(DnsRecord, usize)> {
    let (_, position) = read_name(packet, start)?;
    let type_code = read_u16(packet, position)?;
    let ttl_bytes = packet.get(position + 4..position + 8)?;
    let ttl = u32::from_be_bytes([ttl_bytes[0], ttl_bytes[1], ttl_bytes[2], ttl_bytes[3]]);
    let length = usize::from(read_u16(packet, position + 8)?);
    let data_start = position + 10;
    let data = packet.get(data_start..data_start + length)?;
    let text = match RecordType::from_code(type_code) {
        Some(RecordType::A) => {
            let octets: [u8; 4] = data.try_into().ok()?;
            Ipv4Addr::from(octets).to_string()
        }
        Some(RecordType::Aaaa) => {
            let octets: [u8; 16] = data.try_into().ok()?;
            Ipv6Addr::from(octets).to_string()
        }
        Some(RecordType::Ns | RecordType::Cname | RecordType::Ptr) => {
            read_name(packet, data_start)?.0
        }
        Some(RecordType::Mx) => {
            let preference = read_u16(packet, data_start)?;
            format!("{preference} {}", read_name(packet, data_start + 2)?.0)
        }
        Some(RecordType::Txt) => read_text_strings(data)?,
        None => format!("({length} bytes)"),
    };
    Some((
        DnsRecord {
            type_code,
            ttl,
            data: text,
        },
        data_start + length,
    ))
}

/// The length-prefixed strings of a TXT record, joined into one.
fn read_text_strings(data: &[u8]) -> Option<String> {
    let mut text = String::new();
    let mut position = 0;
    while position < data.len() {
        let length = usize::from(data[position]);
        let piece = data.get(position + 1..position + 1 + length)?;
        text.push_str(&String::from_utf8_lossy(piece));
        position += 1 + length;
    }
    Some(text)
}

/// How one resolver did over the benchmark rounds.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolverTiming {
    pub name: String,
    pub address: IpAddr,
    /// Milliseconds per round, or why a round failed.
    pub rounds: Vec<Result<f64, DnsError>>,
}

impl ResolverTiming {
    pub fn answered(&self) -> usize {
        self.rounds.iter().filter(|round| round.is_ok()).count()
    }

    pub fn average_ms(&self) -> Option<f64> {
        let answered: Vec<f64> = self
            .rounds
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .copied()
            .collect();
        (!answered.is_empty()).then(|| answered.iter().sum::<f64>() / answered.len() as f64)
    }

    /// The reason of the last failed round.
    pub fn last_error(&self) -> Option<&DnsError> {
        self.rounds
            .iter()
            .rev()
            .find_map(|round| round.as_ref().err())
    }
}

/// Times an A lookup of `name` against every resolver at the same time,
/// [`BENCHMARK_ROUNDS`] times each. Results keep the order of `resolvers`.
pub async fn benchmark(resolvers: Vec<(String, IpAddr)>, name: &str) -> Vec<ResolverTiming> {
    let mut tasks = tokio::task::JoinSet::new();
    for (index, (label, address)) in resolvers.into_iter().enumerate() {
        let name = name.to_owned();
        tasks.spawn(async move {
            let mut rounds = Vec::with_capacity(BENCHMARK_ROUNDS);
            for _ in 0..BENCHMARK_ROUNDS {
                let round = lookup(address, &name, RecordType::A)
                    .await
                    .map(|answer| answer.elapsed.as_secs_f64() * 1000.0);
                rounds.push(round);
            }
            (
                index,
                ResolverTiming {
                    name: label,
                    address,
                    rounds,
                },
            )
        });
    }
    let mut timings = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        if let Ok(timing) = joined {
            timings.push(timing);
        }
    }
    timings.sort_by_key(|(index, _)| *index);
    timings.into_iter().map(|(_, timing)| timing).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response to `example.com A` with the given flags and answer records.
    fn response(id: u16, flags: u16, answers: &[Vec<u8>]) -> Vec<u8> {
        let mut packet = Vec::new();
        packet.extend(id.to_be_bytes());
        packet.extend(flags.to_be_bytes());
        packet.extend([0, 1]);
        packet.extend((answers.len() as u16).to_be_bytes());
        packet.extend([0, 0, 0, 0]);
        packet.extend([
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ]);
        packet.extend([0, 1, 0, 1]);
        for answer in answers {
            packet.extend(answer);
        }
        packet
    }

    /// A record whose name points back at the question (offset 12).
    fn record(type_code: u16, ttl: u32, data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xC0, 12];
        bytes.extend(type_code.to_be_bytes());
        bytes.extend(CLASS_IN.to_be_bytes());
        bytes.extend(ttl.to_be_bytes());
        bytes.extend((data.len() as u16).to_be_bytes());
        bytes.extend(data);
        bytes
    }

    const OK: u16 = FLAG_RESPONSE | FLAG_RECURSION_DESIRED;

    #[test]
    fn builds_a_query_with_one_question_and_an_edns_option() {
        let packet = query_packet(0x1234, "example.com", RecordType::Mx);
        assert_eq!(&packet[..2], &[0x12, 0x34]);
        assert_eq!(&packet[2..4], &[0x01, 0x00]);
        assert_eq!(&packet[4..12], &[0, 1, 0, 0, 0, 0, 0, 1]);
        assert_eq!(&packet[12..25], b"\x07example\x03com\x00");
        assert_eq!(&packet[25..29], &[0, 15, 0, 1]);
        assert_eq!(&packet[29..32], &[0, 0, 41]);
    }

    #[test]
    fn reads_address_records() {
        let packet = response(
            7,
            OK,
            &[
                record(1, 300, &[93, 184, 216, 34]),
                record(28, 60, &Ipv6Addr::LOCALHOST.octets()),
            ],
        );
        let parsed = parse_response(&packet, 7).unwrap();
        assert!(!parsed.truncated);
        assert_eq!(parsed.records[0].data, "93.184.216.34");
        assert_eq!(parsed.records[0].ttl, 300);
        assert_eq!(parsed.records[0].type_name(), "A");
        assert_eq!(parsed.records[1].data, "::1");
    }

    #[test]
    fn reads_names_text_and_preferences() {
        let mx = [&[0, 10][..], b"\x04mail\xC0\x0C"].concat();
        let packet = response(
            9,
            OK,
            &[
                record(15, 1, &mx),
                record(16, 1, b"\x06v=spf1\x05 -all"),
                record(5, 1, b"\x03www\xC0\x0C"),
                record(99, 1, &[1, 2, 3]),
            ],
        );
        let records = parse_response(&packet, 9).unwrap().records;
        assert_eq!(records[0].data, "10 mail.example.com");
        assert_eq!(records[1].data, "v=spf1 -all");
        assert_eq!(records[2].data, "www.example.com");
        assert_eq!(records[3].type_name(), "TYPE99");
        assert_eq!(records[3].data, "(3 bytes)");
    }

    #[test]
    fn name_errors_and_failures_are_reported() {
        let not_found = response(1, OK | RCODE_NAME_ERROR, &[]);
        assert_eq!(
            parse_response(&not_found, 1).err(),
            Some(DnsError::NotFound)
        );
        let refused = response(1, OK | 5, &[]);
        assert_eq!(
            parse_response(&refused, 1).err(),
            Some(DnsError::ServerFailure(5))
        );
    }

    #[test]
    fn an_answer_with_no_records_is_not_an_error() {
        let packet = response(2, OK, &[]);
        assert!(parse_response(&packet, 2).unwrap().records.is_empty());
    }

    #[test]
    fn a_cut_short_answer_asks_for_tcp() {
        let packet = response(3, OK | FLAG_TRUNCATED, &[]);
        assert!(parse_response(&packet, 3).unwrap().truncated);
    }

    #[test]
    fn rejects_wrong_ids_queries_and_cut_off_records() {
        let packet = response(4, OK, &[record(1, 1, &[1, 2, 3, 4])]);
        assert_eq!(parse_response(&packet, 5).err(), Some(DnsError::Malformed));
        let mut query_flags = packet.clone();
        query_flags[2] = 0;
        assert_eq!(
            parse_response(&query_flags, 4).err(),
            Some(DnsError::Malformed)
        );
        let cut = &packet[..packet.len() - 2];
        assert_eq!(parse_response(cut, 4).err(), Some(DnsError::Malformed));
        assert_eq!(parse_response(&[], 4).err(), Some(DnsError::Malformed));
    }

    #[test]
    fn builds_reverse_names() {
        assert_eq!(
            reverse_name("192.168.1.7".parse().unwrap()),
            "7.1.168.192.in-addr.arpa"
        );
        assert_eq!(
            reverse_name("2001:db8::1".parse().unwrap()),
            "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa"
        );
    }

    #[test]
    fn a_ptr_question_for_an_address_uses_the_reverse_name() {
        assert_eq!(
            question_name("1.1.1.1", RecordType::Ptr).unwrap(),
            "1.1.1.1.in-addr.arpa"
        );
        assert_eq!(question_name("1.1.1.1", RecordType::A).unwrap(), "1.1.1.1");
    }

    #[test]
    fn checks_names() {
        assert_eq!(
            question_name(" Example.com. ", RecordType::A).unwrap(),
            "Example.com"
        );
        let long_label = "a".repeat(64);
        for bad in [
            "",
            "-x.com",
            "a b.com",
            "a..com",
            "x;y",
            long_label.as_str(),
        ] {
            assert_eq!(
                question_name(bad, RecordType::A),
                Err(DnsError::InvalidName),
                "{bad}"
            );
        }
    }

    #[test]
    fn reads_resolv_conf() {
        let text = "# comment\nnameserver 127.0.0.53\nsearch lan\nnameserver fe80::1%eth0\nnameserver bad\n";
        assert_eq!(
            parse_resolv_conf(text),
            [
                "127.0.0.53".parse::<IpAddr>().unwrap(),
                "fe80::1".parse::<IpAddr>().unwrap()
            ]
        );
    }

    #[test]
    fn timing_summarizes_answered_rounds() {
        let timing = ResolverTiming {
            name: "x".to_owned(),
            address: "1.1.1.1".parse().unwrap(),
            rounds: vec![Ok(10.0), Err(DnsError::Timeout), Ok(20.0)],
        };
        assert_eq!(timing.answered(), 2);
        assert_eq!(timing.average_ms(), Some(15.0));
        assert_eq!(timing.last_error(), Some(&DnsError::Timeout));
    }

    #[tokio::test]
    async fn a_lookup_with_a_bad_name_fails_before_any_network_use() {
        let result = lookup("127.0.0.1".parse().unwrap(), "-bad", RecordType::A).await;
        assert_eq!(result.err(), Some(DnsError::InvalidName));
    }
}
