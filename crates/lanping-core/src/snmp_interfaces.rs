//! Traffic of each network interface of a router or switch, over read-only SNMP v2c.
//!
//! SNMP counts bytes per interface or port, not per device on the network. Nothing is
//! ever written. The counters only grow, so a rate comes from two readings.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::sync::SemaphorePermit;
use tokio::time::timeout;

use crate::snmp::{
    SNMP_PORT, SnmpCommunity, TAG_INTEGER, TAG_NULL, TAG_OCTET_STRING, TAG_OID, TAG_RESPONSE,
    TAG_SEQUENCE, VERSION_2C, clean_text, encode, encode_oid, read_element,
};
use crate::socket_budget::socket_permit;

const TAG_GET_REQUEST: u8 = 0xA0;
const TAG_GET_BULK_REQUEST: u8 = 0xA5;
const TAG_COUNTER32: u8 = 0x41;
const TAG_GAUGE32: u8 = 0x42;
const TAG_TIMETICKS: u8 = 0x43;
const TAG_COUNTER64: u8 = 0x46;
const TAG_NO_SUCH_OBJECT: u8 = 0x80;
const TAG_NO_SUCH_INSTANCE: u8 = 0x81;
const TAG_END_OF_MIB_VIEW: u8 = 0x82;

const ANSWER_WAIT: Duration = Duration::from_secs(2);
const ATTEMPTS: usize = 2;
/// Rows asked for in one bulk request.
const BULK_ROWS: u32 = 20;
const MAX_INTERFACES: usize = 512;
/// Values asked for in one GET, so a request fits in one packet.
const GET_CHUNK: usize = 24;
const RECEIVE_BUFFER: usize = 8192;
/// Samples kept per interface for the graph.
pub const HISTORY_LENGTH: usize = 40;
const BITS_PER_BYTE: f64 = 8.0;
const COUNTER32_RANGE: u64 = 1 << 32;
const TICKS_PER_SECOND: u64 = 100;
const MEGABITS: u64 = 1_000_000;
const OPER_STATUS_UP: i64 = 1;
/// `ifType` of software loopback interfaces.
pub const IF_TYPE_LOOPBACK: u64 = 24;

const SYS_UPTIME: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 3, 0];
const IF_TABLE: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 2, 1];
const IF_X_TABLE: &[u32] = &[1, 3, 6, 1, 2, 1, 31, 1, 1, 1];
/// Columns of `ifTable` (column 2 is the description) and `ifXTable`.
const COLUMN_DESCR: u32 = 2;
const COLUMN_TYPE: u32 = 3;
const COLUMN_SPEED: u32 = 5;
const COLUMN_OPER_STATUS: u32 = 8;
const COLUMN_IN_OCTETS: u32 = 10;
const COLUMN_OUT_OCTETS: u32 = 16;
const COLUMN_X_NAME: u32 = 1;
const COLUMN_X_HC_IN_OCTETS: u32 = 6;
const COLUMN_X_HC_OUT_OCTETS: u32 = 10;
const COLUMN_X_HIGH_SPEED: u32 = 15;
const COLUMN_X_ALIAS: u32 = 18;

fn column(table: &[u32], column: u32) -> Vec<u32> {
    let mut oid = table.to_vec();
    oid.push(column);
    oid
}

fn cell(table: &[u32], column_number: u32, index: u32) -> Vec<u32> {
    let mut oid = column(table, column_number);
    oid.push(index);
    oid
}

/// Why a query got nowhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnmpError {
    /// Nothing answered, which is also what a wrong community looks like.
    NoAnswer,
    /// The device answered with an error code.
    Refused(i64),
    Malformed,
    NoInterfaces,
    Network(String),
}

impl fmt::Display for SnmpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAnswer => f.write_str(
                "No answer. SNMP may be off on that device, a firewall may block UDP port 161, or the community is wrong.",
            ),
            Self::Refused(code) => write!(f, "The device refused the request (error {code})"),
            Self::Malformed => f.write_str("The device sent an answer that could not be read"),
            Self::NoInterfaces => f.write_str("The device answered but lists no interfaces"),
            Self::Network(reason) => write!(f, "Could not send the request: {reason}"),
        }
    }
}

/// One value of an answer.
#[derive(Debug, Clone, PartialEq)]
enum Value {
    Integer(i64),
    /// Counters, gauges and time ticks.
    Unsigned(u64),
    Text(String),
    /// `noSuchObject`, `noSuchInstance` or `endOfMibView`.
    Missing,
    Other,
}

fn decode_oid(bytes: &[u8]) -> Vec<u32> {
    let mut oid = Vec::new();
    let mut groups = bytes.iter();
    if let Some(&first) = groups.next() {
        let top = (first / 40).min(2);
        oid.extend([u32::from(top), u32::from(first) - u32::from(top) * 40]);
    }
    let mut part = 0_u32;
    for &byte in groups {
        part = (part << 7) | u32::from(byte & 0x7F);
        if byte & 0x80 == 0 {
            oid.push(part);
            part = 0;
        }
    }
    oid
}

/// Big-endian unsigned value. Leading zero bytes, which keep a counter from reading as
/// negative, are skipped.
fn unsigned(bytes: &[u8]) -> Option<u64> {
    let first = bytes
        .iter()
        .position(|byte| *byte != 0)
        .unwrap_or(bytes.len());
    let bytes = &bytes[first..];
    (bytes.len() <= 8).then(|| {
        bytes
            .iter()
            .fold(0, |total, byte| (total << 8) | u64::from(*byte))
    })
}

fn signed(bytes: &[u8]) -> Option<i64> {
    if bytes.is_empty() || bytes.len() > 8 {
        return None;
    }
    let negative = bytes[0] & 0x80 != 0;
    Some(
        bytes
            .iter()
            .fold(if negative { -1_i64 } else { 0 }, |total, byte| {
                (total << 8) | i64::from(*byte)
            }),
    )
}

fn read_value(kind: u8, bytes: &[u8]) -> Value {
    match kind {
        TAG_INTEGER => signed(bytes).map_or(Value::Other, Value::Integer),
        TAG_COUNTER64 => unsigned(bytes).map_or(Value::Other, Value::Unsigned),
        // These types are 32 bits wide. A larger number is a broken device, and one that
        // would make the wrap arithmetic overflow.
        TAG_COUNTER32 | TAG_GAUGE32 | TAG_TIMETICKS => unsigned(bytes)
            .filter(|value| *value <= u64::from(u32::MAX))
            .map_or(Value::Other, Value::Unsigned),
        TAG_OCTET_STRING => {
            clean_text(bytes).map_or_else(|| Value::Text(String::new()), Value::Text)
        }
        TAG_NO_SUCH_OBJECT | TAG_NO_SUCH_INSTANCE | TAG_END_OF_MIB_VIEW => Value::Missing,
        _ => Value::Other,
    }
}

fn encode_uint(value: u32) -> Vec<u8> {
    let mut bytes: Vec<u8> = value
        .to_be_bytes()
        .into_iter()
        .skip_while(|b| *b == 0)
        .collect();
    if bytes.first().is_none_or(|first| first & 0x80 != 0) {
        bytes.insert(0, 0);
    }
    encode(TAG_INTEGER, &bytes)
}

/// A GET or GETBULK request. `first` and `second` are the error fields of a GET, which are
/// zero, or the non-repeaters and repetitions of a GETBULK.
fn request(
    tag: u8,
    id: u32,
    community: &str,
    first: u32,
    second: u32,
    oids: &[Vec<u32>],
) -> Vec<u8> {
    let bindings: Vec<u8> = oids
        .iter()
        .flat_map(|oid| {
            let mut binding = encode_oid(oid);
            binding.extend(encode(TAG_NULL, &[]));
            encode(TAG_SEQUENCE, &binding)
        })
        .collect();
    let mut pdu = encode_uint(id);
    pdu.extend(encode_uint(first));
    pdu.extend(encode_uint(second));
    pdu.extend(encode(TAG_SEQUENCE, &bindings));
    let mut message = encode(TAG_INTEGER, &[VERSION_2C]);
    message.extend(encode(TAG_OCTET_STRING, community.as_bytes()));
    message.extend(encode(tag, &pdu));
    encode(TAG_SEQUENCE, &message)
}

/// What came back for a request.
enum Answer {
    Values(Vec<(Vec<u32>, Value)>),
    /// An answer to another request.
    Other,
}

fn parse_response(packet: &[u8], expected_id: u32) -> Result<Answer, SnmpError> {
    let malformed = || SnmpError::Malformed;
    let (TAG_SEQUENCE, message, _) = read_element(packet).ok_or_else(malformed)? else {
        return Err(malformed());
    };
    let (TAG_INTEGER, _, rest) = read_element(message).ok_or_else(malformed)? else {
        return Err(malformed());
    };
    let (TAG_OCTET_STRING, _, rest) = read_element(rest).ok_or_else(malformed)? else {
        return Err(malformed());
    };
    let (TAG_RESPONSE, pdu, _) = read_element(rest).ok_or_else(malformed)? else {
        return Err(malformed());
    };
    let (TAG_INTEGER, id, rest) = read_element(pdu).ok_or_else(malformed)? else {
        return Err(malformed());
    };
    if unsigned(id) != Some(u64::from(expected_id)) {
        return Ok(Answer::Other);
    }
    let (TAG_INTEGER, status, rest) = read_element(rest).ok_or_else(malformed)? else {
        return Err(malformed());
    };
    let (_, _, rest) = read_element(rest).ok_or_else(malformed)?;
    let status = signed(status).ok_or_else(malformed)?;
    if status != 0 {
        return Err(SnmpError::Refused(status));
    }
    let (TAG_SEQUENCE, mut bindings, _) = read_element(rest).ok_or_else(malformed)? else {
        return Err(malformed());
    };
    let mut values = Vec::new();
    while let Some((TAG_SEQUENCE, binding, next)) = read_element(bindings) {
        bindings = next;
        let (TAG_OID, oid, after) = read_element(binding).ok_or_else(malformed)? else {
            return Err(malformed());
        };
        let (kind, value, _) = read_element(after).ok_or_else(malformed)?;
        values.push((decode_oid(oid), read_value(kind, value)));
    }
    Ok(Answer::Values(values))
}

fn next_request_id() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed) & 0x7FFF_FFFF
}

/// One socket to one device, for a series of requests.
struct Session {
    socket: UdpSocket,
    community: String,
    wait: Duration,
    _permit: Option<SemaphorePermit<'static>>,
}

impl Session {
    async fn open(
        address: SocketAddrV4,
        community: &SnmpCommunity,
        wait: Duration,
    ) -> Result<Self, SnmpError> {
        let permit = socket_permit().await;
        let network = |error: std::io::Error| SnmpError::Network(error.to_string());
        let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
            .await
            .map_err(network)?;
        socket
            .connect(SocketAddr::V4(address))
            .await
            .map_err(network)?;
        Ok(Self {
            socket,
            community: community.0.clone(),
            wait,
            _permit: permit,
        })
    }

    /// Sends a request and waits for its answer, trying again once if nothing comes.
    async fn exchange(
        &self,
        tag: u8,
        first: u32,
        second: u32,
        oids: &[Vec<u32>],
    ) -> Result<Vec<(Vec<u32>, Value)>, SnmpError> {
        let id = next_request_id();
        let packet = request(tag, id, &self.community, first, second, oids);
        let mut buffer = vec![0; RECEIVE_BUFFER];
        for _ in 0..ATTEMPTS {
            self.socket
                .send(&packet)
                .await
                .map_err(|error| SnmpError::Network(error.to_string()))?;
            let deadline = Instant::now() + self.wait;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                let Ok(Ok(length)) = timeout(left, self.socket.recv(&mut buffer)).await else {
                    break;
                };
                match parse_response(&buffer[..length], id) {
                    Ok(Answer::Values(values)) => return Ok(values),
                    // Another request's answer, or a stray datagram: keep waiting for ours.
                    Ok(Answer::Other) | Err(SnmpError::Malformed) => {}
                    Err(error) => return Err(error),
                }
            }
        }
        Err(SnmpError::NoAnswer)
    }

    async fn get(&self, oids: &[Vec<u32>]) -> Result<Vec<(Vec<u32>, Value)>, SnmpError> {
        self.exchange(TAG_GET_REQUEST, 0, 0, oids).await
    }

    /// Every row of one table column, as the index and the value. Stops at the end of the
    /// column, or after [`MAX_INTERFACES`] rows.
    async fn walk(&self, base: &[u32]) -> Result<Vec<(u32, Value)>, SnmpError> {
        let mut rows = Vec::new();
        let mut last = base.to_vec();
        while rows.len() < MAX_INTERFACES {
            let answer = self
                .exchange(
                    TAG_GET_BULK_REQUEST,
                    0,
                    BULK_ROWS,
                    std::slice::from_ref(&last),
                )
                .await?;
            let mut progressed = false;
            for (oid, value) in answer {
                let in_column = oid.len() == base.len() + 1 && oid.starts_with(base);
                if !in_column || value == Value::Missing {
                    return Ok(rows);
                }
                rows.push((oid[base.len()], value));
                last = oid;
                progressed = true;
            }
            if !progressed {
                break;
            }
        }
        Ok(rows)
    }
}

/// An interface of the device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    /// `ifIndex`, which names the interface in every table.
    pub index: u32,
    pub name: String,
    /// What the administrator called it, when set.
    pub alias: Option<String>,
    /// `ifType`; 24 is a software loopback.
    pub kind: u64,
    pub speed_mbps: Option<u64>,
    pub up: bool,
    /// Whether the 64-bit counters exist. Otherwise the 32-bit ones are read, which wrap quickly.
    pub wide: bool,
}

impl Interface {
    pub fn is_loopback(&self) -> bool {
        self.kind == IF_TYPE_LOOPBACK
    }
}

/// What one interface had counted at one reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reading {
    pub in_octets: u64,
    pub out_octets: u64,
    pub up: bool,
    pub wide: bool,
}

/// All readings taken together.
#[derive(Debug, Clone, PartialEq)]
pub struct Poll {
    pub taken: Instant,
    /// The device's uptime in hundredths of a second. A drop means it restarted.
    pub uptime_ticks: Option<u64>,
    pub readings: HashMap<u32, Reading>,
}

/// A device to ask.
#[derive(Debug, Clone)]
pub struct Agent {
    address: SocketAddrV4,
    community: SnmpCommunity,
    wait: Duration,
}

impl Agent {
    pub fn new(ip: Ipv4Addr, community: SnmpCommunity) -> Self {
        Self {
            address: SocketAddrV4::new(ip, SNMP_PORT),
            community,
            wait: ANSWER_WAIT,
        }
    }

    /// Asks on `port` instead of the standard 161, for a device or test agent that listens elsewhere.
    pub fn with_port(mut self, port: u16) -> Self {
        self.address.set_port(port);
        self
    }

    /// Lists the interfaces with their names, speeds and which counters they have.
    pub async fn discover(&self) -> Result<Vec<Interface>, SnmpError> {
        let session = Session::open(self.address, &self.community, self.wait).await?;
        let descriptions = session.walk(&column(IF_TABLE, COLUMN_DESCR)).await?;
        if descriptions.is_empty() {
            return Err(SnmpError::NoInterfaces);
        }
        let names = by_index(session.walk(&column(IF_X_TABLE, COLUMN_X_NAME)).await?);
        let aliases = by_index(session.walk(&column(IF_X_TABLE, COLUMN_X_ALIAS)).await?);
        let kinds = by_index(session.walk(&column(IF_TABLE, COLUMN_TYPE)).await?);
        let statuses = by_index(session.walk(&column(IF_TABLE, COLUMN_OPER_STATUS)).await?);
        let speeds = by_index(session.walk(&column(IF_TABLE, COLUMN_SPEED)).await?);
        let high_speeds = by_index(
            session
                .walk(&column(IF_X_TABLE, COLUMN_X_HIGH_SPEED))
                .await?,
        );
        let wide = by_index(
            session
                .walk(&column(IF_X_TABLE, COLUMN_X_HC_IN_OCTETS))
                .await?,
        );
        Ok(descriptions
            .into_iter()
            .map(|(index, description)| {
                let text = |map: &HashMap<u32, Value>| match map.get(&index) {
                    Some(Value::Text(text)) if !text.is_empty() => Some(text.clone()),
                    _ => None,
                };
                let number = |map: &HashMap<u32, Value>| match map.get(&index) {
                    Some(Value::Unsigned(value)) => Some(*value),
                    Some(Value::Integer(value)) => u64::try_from(*value).ok(),
                    _ => None,
                };
                let description = match description {
                    Value::Text(text) if !text.is_empty() => Some(text),
                    _ => None,
                };
                let name = text(&names)
                    .or(description)
                    .unwrap_or_else(|| format!("Interface {index}"));
                Interface {
                    index,
                    name,
                    alias: text(&aliases),
                    kind: number(&kinds).unwrap_or(0),
                    speed_mbps: speed_in_megabits(number(&high_speeds), number(&speeds)),
                    up: matches!(statuses.get(&index), Some(Value::Integer(status)) if *status == OPER_STATUS_UP),
                    wide: matches!(wide.get(&index), Some(Value::Unsigned(_))),
                }
            })
            .collect())
    }

    /// Reads the counters and status of `interfaces`, and the device's uptime.
    pub async fn poll(&self, interfaces: &[Interface]) -> Result<Poll, SnmpError> {
        let session = Session::open(self.address, &self.community, self.wait).await?;
        let mut wanted: Vec<Vec<u32>> = vec![SYS_UPTIME.to_vec()];
        for interface in interfaces {
            let (table, in_column, out_column) = counter_columns(interface.wide);
            wanted.push(cell(IF_TABLE, COLUMN_OPER_STATUS, interface.index));
            wanted.push(cell(table, in_column, interface.index));
            wanted.push(cell(table, out_column, interface.index));
        }
        let mut values: HashMap<Vec<u32>, Value> = HashMap::new();
        for chunk in wanted.chunks(GET_CHUNK) {
            values.extend(session.get(chunk).await?);
        }
        let taken = Instant::now();
        let uptime_ticks = match values.get(SYS_UPTIME) {
            Some(Value::Unsigned(ticks)) => Some(*ticks),
            _ => None,
        };
        let readings = interfaces
            .iter()
            .filter_map(|interface| {
                let counter = |table: &[u32], number: u32| match values.get(&cell(
                    table,
                    number,
                    interface.index,
                )) {
                    Some(Value::Unsigned(value)) => Some(*value),
                    _ => None,
                };
                let (table, in_column, out_column) = counter_columns(interface.wide);
                let up = matches!(
                    values.get(&cell(IF_TABLE, COLUMN_OPER_STATUS, interface.index)),
                    Some(Value::Integer(status)) if *status == OPER_STATUS_UP
                );
                Some((
                    interface.index,
                    Reading {
                        in_octets: counter(table, in_column)?,
                        out_octets: counter(table, out_column)?,
                        up,
                        wide: interface.wide,
                    },
                ))
            })
            .collect();
        Ok(Poll {
            taken,
            uptime_ticks,
            readings,
        })
    }
}

/// The table and the in and out columns that hold an interface's byte counters: the 64-bit
/// ones when it has them, otherwise the 32-bit ones.
fn counter_columns(wide: bool) -> (&'static [u32], u32, u32) {
    if wide {
        (IF_X_TABLE, COLUMN_X_HC_IN_OCTETS, COLUMN_X_HC_OUT_OCTETS)
    } else {
        (IF_TABLE, COLUMN_IN_OCTETS, COLUMN_OUT_OCTETS)
    }
}

fn by_index(rows: Vec<(u32, Value)>) -> HashMap<u32, Value> {
    rows.into_iter().collect()
}

/// The link speed in megabits per second. `ifHighSpeed` is already in megabits; `ifSpeed` is
/// in bits and stops at about 4.29 Gbit/s, where it reports the largest number it can.
fn speed_in_megabits(high_speed: Option<u64>, speed: Option<u64>) -> Option<u64> {
    match (high_speed, speed) {
        (Some(megabits), _) if megabits > 0 => Some(megabits),
        (_, Some(bits)) if bits > 0 => Some(bits / MEGABITS),
        _ => None,
    }
}

/// Bits per second in and out of one interface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub in_bps: f64,
    pub out_bps: f64,
}

impl Rate {
    pub fn total_bps(self) -> f64 {
        self.in_bps + self.out_bps
    }
}

/// How much a counter grew. 32-bit counters wrap around, so a smaller number means a wrap.
/// A 64-bit counter never wraps in practice, so a smaller number means it was reset.
fn counter_growth(before: u64, now: u64, wide: bool) -> Option<u64> {
    if wide {
        now.checked_sub(before)
    } else {
        Some(now.wrapping_sub(before) & (COUNTER32_RANGE - 1))
    }
}

/// Turns readings into rates and keeps recent ones for a graph.
#[derive(Debug, Default)]
pub struct RateTracker {
    previous: Option<Poll>,
    latest: HashMap<u32, Rate>,
    history: HashMap<u32, VecDeque<Rate>>,
}

impl RateTracker {
    /// Adds a reading. The first one gives no rates; each later one gives the rates since the
    /// one before. A device that restarted, or an interface whose counter was reset, gives
    /// none for that step.
    pub fn record(&mut self, poll: Poll) {
        let restarted = match (&self.previous, poll.uptime_ticks) {
            (Some(previous), Some(now)) => previous.uptime_ticks.is_some_and(|before| now < before),
            _ => false,
        };
        self.latest.clear();
        if let Some(previous) = self.previous.as_ref().filter(|_| !restarted) {
            let seconds = poll.taken.duration_since(previous.taken).as_secs_f64();
            if seconds > 0.0 {
                for (index, now) in &poll.readings {
                    let Some(before) = previous.readings.get(index) else {
                        continue;
                    };
                    let (Some(in_growth), Some(out_growth)) = (
                        counter_growth(before.in_octets, now.in_octets, now.wide),
                        counter_growth(before.out_octets, now.out_octets, now.wide),
                    ) else {
                        continue;
                    };
                    let rate = Rate {
                        in_bps: in_growth as f64 * BITS_PER_BYTE / seconds,
                        out_bps: out_growth as f64 * BITS_PER_BYTE / seconds,
                    };
                    self.latest.insert(*index, rate);
                    let history = self.history.entry(*index).or_default();
                    history.push_back(rate);
                    if history.len() > HISTORY_LENGTH {
                        history.pop_front();
                    }
                }
            }
        }
        self.previous = Some(poll);
    }

    /// The rate over the last step, when there was one.
    pub fn rate(&self, index: u32) -> Option<Rate> {
        self.latest.get(&index).copied()
    }

    /// Recent rates, oldest first.
    pub fn history(&self, index: u32) -> Vec<Rate> {
        self.history
            .get(&index)
            .map(|rates| rates.iter().copied().collect())
            .unwrap_or_default()
    }

    /// The device's uptime in seconds at the last reading.
    pub fn uptime_seconds(&self) -> Option<u64> {
        self.previous
            .as_ref()
            .and_then(|poll| poll.uptime_ticks)
            .map(|ticks| ticks / TICKS_PER_SECOND)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A value as the agent sends it: its tag and content.
    #[derive(Clone)]
    struct Raw(u8, Vec<u8>);

    fn integer(value: i64) -> Raw {
        let mut bytes = value.to_be_bytes().to_vec();
        while bytes.len() > 1
            && ((bytes[0] == 0 && bytes[1] & 0x80 == 0)
                || (bytes[0] == 0xFF && bytes[1] & 0x80 != 0))
        {
            bytes.remove(0);
        }
        Raw(TAG_INTEGER, bytes)
    }

    fn unsigned_value(tag: u8, value: u64) -> Raw {
        let mut bytes: Vec<u8> = value
            .to_be_bytes()
            .into_iter()
            .skip_while(|b| *b == 0)
            .collect();
        if bytes.first().is_none_or(|first| first & 0x80 != 0) {
            bytes.insert(0, 0);
        }
        Raw(tag, bytes)
    }

    fn counter64(value: u64) -> Raw {
        unsigned_value(TAG_COUNTER64, value)
    }

    fn counter32(value: u64) -> Raw {
        unsigned_value(TAG_COUNTER32, value)
    }

    fn text(value: &str) -> Raw {
        Raw(TAG_OCTET_STRING, value.as_bytes().to_vec())
    }

    /// A small SNMP agent: answers GET and GETBULK from a table, for one community.
    struct FakeAgent {
        mib: BTreeMap<Vec<u32>, Raw>,
        community: &'static str,
        /// An error status to answer every request with.
        error: i64,
    }

    impl FakeAgent {
        fn reply(&self, packet: &[u8]) -> Option<Vec<u8>> {
            let (_, message, _) = read_element(packet)?;
            let (_, _, rest) = read_element(message)?;
            let (_, community, rest) = read_element(rest)?;
            if community != self.community.as_bytes() {
                return None;
            }
            let (tag, pdu, _) = read_element(rest)?;
            let (_, id, rest) = read_element(pdu)?;
            let (_, first, rest) = read_element(rest)?;
            let (_, second, rest) = read_element(rest)?;
            let (_, mut bindings, _) = read_element(rest)?;
            let repetitions = second
                .iter()
                .fold(0_u32, |total, byte| (total << 8) | u32::from(*byte));
            let mut answers: Vec<(Vec<u32>, Raw)> = Vec::new();
            while let Some((_, binding, next)) = read_element(bindings) {
                bindings = next;
                let (_, oid, _) = read_element(binding)?;
                let oid = decode_oid(oid);
                if tag == TAG_GET_REQUEST {
                    let value = self
                        .mib
                        .get(&oid)
                        .cloned()
                        .unwrap_or(Raw(TAG_NO_SUCH_INSTANCE, vec![]));
                    answers.push((oid, value));
                } else {
                    let after: Vec<_> = self
                        .mib
                        .range::<Vec<u32>, _>((
                            std::ops::Bound::Excluded(&oid),
                            std::ops::Bound::Unbounded,
                        ))
                        .take(repetitions as usize)
                        .map(|(oid, value)| (oid.clone(), value.clone()))
                        .collect();
                    if after.is_empty() {
                        answers.push((oid, Raw(TAG_END_OF_MIB_VIEW, vec![])));
                    }
                    answers.extend(after);
                }
            }
            let _ = first;
            let list: Vec<u8> = answers
                .iter()
                .flat_map(|(oid, Raw(tag, bytes))| {
                    let mut binding = encode_oid(oid);
                    binding.extend(encode(*tag, bytes));
                    encode(TAG_SEQUENCE, &binding)
                })
                .collect();
            let mut response = encode(TAG_INTEGER, id);
            response.extend(encode_uint(self.error as u32));
            response.extend(encode_uint(0));
            response.extend(encode(TAG_SEQUENCE, &list));
            let mut out = encode(TAG_INTEGER, &[VERSION_2C]);
            out.extend(encode(TAG_OCTET_STRING, community));
            out.extend(encode(TAG_RESPONSE, &response));
            Some(encode(TAG_SEQUENCE, &out))
        }
    }

    /// Starts an agent and returns an [`Agent`] pointing at it.
    async fn serve(agent: FakeAgent) -> Agent {
        let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let SocketAddr::V4(address) = socket.local_addr().unwrap() else {
            panic!("bound to IPv4");
        };
        tokio::spawn(async move {
            let mut buffer = vec![0; 8192];
            while let Ok((length, from)) = socket.recv_from(&mut buffer).await {
                if let Some(reply) = agent.reply(&buffer[..length]) {
                    let _ = socket.send_to(&reply, from).await;
                }
            }
        });
        Agent {
            address,
            community: SnmpCommunity("public".to_owned()),
            wait: Duration::from_millis(200),
        }
    }

    /// A router: a loopback, a busy 1 Gbit/s port with 64-bit counters, and an idle one that is
    /// down and has only 32-bit counters.
    fn router_mib() -> BTreeMap<Vec<u32>, Raw> {
        let mut mib = BTreeMap::new();
        let mut put = |oid: Vec<u32>, value: Raw| {
            mib.insert(oid, value);
        };
        put(
            SYS_UPTIME.to_vec(),
            Raw(TAG_TIMETICKS, vec![0x01, 0x00, 0x00]),
        );
        for (index, descr, kind, status, speed) in [
            (1, "lo", 24, 1, 0_u64),
            (2, "eth0", 6, 1, 1_000_000_000),
            (3, "eth1", 6, 2, 100_000_000),
        ] {
            put(cell(IF_TABLE, COLUMN_DESCR, index), text(descr));
            put(cell(IF_TABLE, COLUMN_TYPE, index), integer(kind));
            put(cell(IF_TABLE, COLUMN_OPER_STATUS, index), integer(status));
            put(
                cell(IF_TABLE, COLUMN_SPEED, index),
                unsigned_value(TAG_GAUGE32, speed),
            );
            put(
                cell(IF_TABLE, COLUMN_IN_OCTETS, index),
                counter32(1000 * u64::from(index)),
            );
            put(
                cell(IF_TABLE, COLUMN_OUT_OCTETS, index),
                counter32(500 * u64::from(index)),
            );
        }
        for index in [1, 2] {
            put(
                cell(IF_X_TABLE, COLUMN_X_NAME, index),
                text(if index == 1 { "lo" } else { "ge-0/0/0" }),
            );
            put(
                cell(IF_X_TABLE, COLUMN_X_HC_IN_OCTETS, index),
                counter64(10_000_000_000 * u64::from(index)),
            );
            put(
                cell(IF_X_TABLE, COLUMN_X_HC_OUT_OCTETS, index),
                counter64(5_000_000_000 * u64::from(index)),
            );
        }
        put(
            cell(IF_X_TABLE, COLUMN_X_HIGH_SPEED, 2),
            unsigned_value(TAG_GAUGE32, 1000),
        );
        put(cell(IF_X_TABLE, COLUMN_X_ALIAS, 2), text("Uplink to ISP"));
        mib
    }

    fn router() -> FakeAgent {
        FakeAgent {
            mib: router_mib(),
            community: "public",
            error: 0,
        }
    }

    #[test]
    fn object_identifiers_round_trip_including_large_parts() {
        for oid in [vec![1, 3, 6, 1, 2, 1, 1, 3, 0], vec![1, 3, 300, 70_000, 1]] {
            let encoded = encode_oid(&oid);
            assert_eq!(decode_oid(&encoded[2..]), oid);
        }
    }

    #[test]
    fn reads_signed_and_unsigned_numbers_including_a_leading_zero() {
        assert_eq!(
            unsigned(&[0, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
            Some(u64::MAX)
        );
        assert_eq!(unsigned(&[0x01, 0x00]), Some(256));
        assert_eq!(unsigned(&[0; 10]), Some(0));
        assert_eq!(unsigned(&[1; 9]), None);
        assert_eq!(signed(&[0xFF]), Some(-1));
        assert_eq!(signed(&[0x01, 0x00]), Some(256));
        assert_eq!(signed(&[]), None);
    }

    #[test]
    fn requests_carry_the_request_id_and_bulk_fields() {
        let packet = request(
            TAG_GET_BULK_REQUEST,
            300,
            "public",
            0,
            20,
            &[IF_TABLE.to_vec()],
        );
        let (_, message, _) = read_element(&packet).unwrap();
        let (_, _, rest) = read_element(message).unwrap();
        let (_, community, rest) = read_element(rest).unwrap();
        assert_eq!(community, b"public");
        let (tag, pdu, _) = read_element(rest).unwrap();
        assert_eq!(tag, TAG_GET_BULK_REQUEST);
        let (_, id, rest) = read_element(pdu).unwrap();
        assert_eq!(id, [0x01, 0x2C]);
        let (_, non_repeaters, rest) = read_element(rest).unwrap();
        assert_eq!(non_repeaters, [0]);
        let (_, repetitions, _) = read_element(rest).unwrap();
        assert_eq!(repetitions, [20]);
    }

    #[test]
    fn a_32_bit_counter_with_a_number_too_big_for_it_is_not_a_number() {
        let too_big = [1, 0, 0, 0, 0];
        assert_eq!(read_value(TAG_COUNTER32, &too_big), Value::Other);
        assert_eq!(read_value(TAG_GAUGE32, &too_big), Value::Other);
        assert!(matches!(
            read_value(TAG_COUNTER64, &too_big),
            Value::Unsigned(_)
        ));
        assert_eq!(
            counter_growth(10, u64::MAX, false),
            Some(u64::from(u32::MAX) - 10)
        );
    }

    #[test]
    fn counters_grow_with_wraps_for_32_bit_and_resets_for_64_bit() {
        assert_eq!(counter_growth(100, 250, false), Some(150));
        assert_eq!(counter_growth(COUNTER32_RANGE - 10, 20, false), Some(30));
        assert_eq!(counter_growth(100, 250, true), Some(150));
        assert_eq!(counter_growth(250, 100, true), None);
    }

    #[test]
    fn link_speed_prefers_the_high_speed_counter_in_megabits() {
        assert_eq!(
            speed_in_megabits(Some(10_000), Some(u64::from(u32::MAX))),
            Some(10_000)
        );
        assert_eq!(speed_in_megabits(Some(0), Some(100_000_000)), Some(100));
        assert_eq!(speed_in_megabits(None, None), None);
        assert_eq!(speed_in_megabits(Some(0), Some(0)), None);
    }

    #[tokio::test]
    async fn discovers_the_interfaces_of_a_router() {
        let agent = serve(router()).await;
        let interfaces = agent.discover().await.unwrap();
        assert_eq!(interfaces.len(), 3);
        let [lo, eth0, eth1] = &interfaces[..] else {
            panic!("expected three interfaces");
        };
        assert!(lo.is_loopback() && lo.up);
        assert_eq!(eth0.name, "ge-0/0/0", "ifName is preferred over ifDescr");
        assert_eq!(eth0.alias.as_deref(), Some("Uplink to ISP"));
        assert_eq!(eth0.speed_mbps, Some(1000));
        assert!(eth0.up && eth0.wide);
        assert_eq!(eth1.name, "eth1");
        assert_eq!(eth1.speed_mbps, Some(100));
        assert!(
            !eth1.up && !eth1.wide,
            "no 64-bit counters, so the 32-bit ones are read"
        );
    }

    #[tokio::test]
    async fn polls_wide_and_narrow_counters_and_status() {
        let agent = serve(router()).await;
        let interfaces = agent.discover().await.unwrap();
        let poll = agent.poll(&interfaces).await.unwrap();
        assert_eq!(poll.uptime_ticks, Some(65_536));
        assert_eq!(poll.readings[&2].in_octets, 20_000_000_000);
        assert_eq!(poll.readings[&2].out_octets, 10_000_000_000);
        assert!(poll.readings[&2].wide && poll.readings[&2].up);
        assert_eq!(poll.readings[&3].in_octets, 3000);
        assert!(!poll.readings[&3].wide && !poll.readings[&3].up);
    }

    #[tokio::test]
    async fn a_wrong_community_gets_no_answer() {
        let mut agent = serve(router()).await;
        agent.community = SnmpCommunity("wrong".to_owned());
        assert_eq!(agent.discover().await.unwrap_err(), SnmpError::NoAnswer);
    }

    #[tokio::test]
    async fn an_error_status_is_reported() {
        let mut busy = router();
        busy.error = 5;
        let agent = serve(busy).await;
        assert_eq!(agent.discover().await.unwrap_err(), SnmpError::Refused(5));
    }

    #[tokio::test]
    async fn a_device_with_no_interface_table_says_so() {
        let agent = serve(FakeAgent {
            mib: BTreeMap::new(),
            community: "public",
            error: 0,
        })
        .await;
        assert_eq!(agent.discover().await.unwrap_err(), SnmpError::NoInterfaces);
    }

    fn poll_at(taken: Instant, ticks: u64, readings: &[(u32, Reading)]) -> Poll {
        Poll {
            taken,
            uptime_ticks: Some(ticks),
            readings: readings.iter().copied().collect(),
        }
    }

    fn reading(in_octets: u64, out_octets: u64, wide: bool) -> Reading {
        Reading {
            in_octets,
            out_octets,
            up: true,
            wide,
        }
    }

    #[test]
    fn rates_come_from_two_readings() {
        let start = Instant::now();
        let mut tracker = RateTracker::default();
        tracker.record(poll_at(start, 1000, &[(2, reading(0, 0, true))]));
        assert_eq!(tracker.rate(2), None, "one reading gives no rate");
        // 25 MB in and 12.5 MB out over 2 seconds.
        tracker.record(poll_at(
            start + Duration::from_secs(2),
            1200,
            &[(2, reading(25_000_000, 12_500_000, true))],
        ));
        let rate = tracker.rate(2).unwrap();
        assert_eq!(rate.in_bps, 100_000_000.0);
        assert_eq!(rate.out_bps, 50_000_000.0);
        assert_eq!(rate.total_bps(), 150_000_000.0);
        assert_eq!(tracker.history(2).len(), 1);
        assert_eq!(tracker.uptime_seconds(), Some(12));
    }

    #[test]
    fn a_32_bit_wrap_is_counted_and_a_restart_gives_no_rate() {
        let start = Instant::now();
        let mut tracker = RateTracker::default();
        let near_wrap = COUNTER32_RANGE - 1000;
        tracker.record(poll_at(start, 1000, &[(3, reading(near_wrap, 0, false))]));
        tracker.record(poll_at(
            start + Duration::from_secs(1),
            1100,
            &[(3, reading(3000, 0, false))],
        ));
        assert_eq!(tracker.rate(3).unwrap().in_bps, 4000.0 * 8.0);
        // The device restarted: its uptime dropped and its counters began again.
        tracker.record(poll_at(
            start + Duration::from_secs(2),
            50,
            &[(3, reading(10, 0, false))],
        ));
        assert_eq!(tracker.rate(3), None);
        tracker.record(poll_at(
            start + Duration::from_secs(3),
            150,
            &[(3, reading(1010, 0, false))],
        ));
        assert_eq!(tracker.rate(3).unwrap().in_bps, 8000.0);
    }

    #[test]
    fn a_reset_64_bit_counter_is_skipped_and_history_is_bounded() {
        let start = Instant::now();
        let mut tracker = RateTracker::default();
        tracker.record(poll_at(start, 100, &[(2, reading(5000, 5000, true))]));
        tracker.record(poll_at(
            start + Duration::from_secs(1),
            200,
            &[(2, reading(10, 10, true))],
        ));
        assert_eq!(tracker.rate(2), None);
        let mut octets = 0;
        for step in 2..HISTORY_LENGTH as u64 + 10 {
            octets += 1000;
            tracker.record(poll_at(
                start + Duration::from_secs(step),
                200 + step,
                &[(2, reading(octets, octets, true))],
            ));
        }
        assert_eq!(tracker.history(2).len(), HISTORY_LENGTH);
    }

    #[test]
    fn errors_say_what_to_check() {
        assert!(SnmpError::NoAnswer.to_string().contains("community"));
        assert!(SnmpError::NoAnswer.to_string().contains("161"));
    }
}
