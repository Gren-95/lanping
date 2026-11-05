//! Read-only SNMP v2c queries for a device's own description. Nothing is ever written.
//! BER is encoded and decoded here by hand, for only the few types these queries use.

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;
use tokio::time::timeout;

use crate::socket_budget::socket_permit;

pub(crate) const SNMP_PORT: u16 = 161;
pub(crate) const VERSION_2C: u8 = 1;
pub(crate) const TAG_INTEGER: u8 = 0x02;
pub(crate) const TAG_OCTET_STRING: u8 = 0x04;
pub(crate) const TAG_NULL: u8 = 0x05;
pub(crate) const TAG_OID: u8 = 0x06;
pub(crate) const TAG_SEQUENCE: u8 = 0x30;
const TAG_GET_REQUEST: u8 = 0xA0;
pub(crate) const TAG_RESPONSE: u8 = 0xA2;
pub(crate) const TAG_TIMETICKS: u8 = 0x43;
const TICKS_PER_SECOND: u64 = 100;
const MAX_TEXT_CHARS: usize = 200;
const REQUEST_ID: u8 = 0x4C;

const SYS_DESCR: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 1, 0];
const SYS_UPTIME: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 3, 0];
const SYS_NAME: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 5, 0];
const SYS_LOCATION: &[u32] = &[1, 3, 6, 1, 2, 1, 1, 6, 0];
const IF_NUMBER: &[u32] = &[1, 3, 6, 1, 2, 1, 2, 1, 0];
const ENTITY_SERIAL: &[u32] = &[1, 3, 6, 1, 2, 1, 47, 1, 1, 1, 1, 11, 1];
const ENTITY_MODEL: &[u32] = &[1, 3, 6, 1, 2, 1, 47, 1, 1, 1, 1, 13, 1];
const QUERIED: [&[u32]; 7] = [
    SYS_DESCR,
    SYS_UPTIME,
    SYS_NAME,
    SYS_LOCATION,
    IF_NUMBER,
    ENTITY_SERIAL,
    ENTITY_MODEL,
];

/// An SNMP community. It works like a password, so debug output hides it.
#[derive(Clone, PartialEq, Eq)]
pub struct SnmpCommunity(pub String);

impl fmt::Debug for SnmpCommunity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SnmpCommunity(hidden)")
    }
}

/// What a device reports about itself over SNMP.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnmpInfo {
    pub description: Option<String>,
    pub name: Option<String>,
    pub location: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
    pub uptime_seconds: Option<u64>,
    /// How many network interfaces or switch ports the device lists.
    pub interface_count: Option<u64>,
}

pub(crate) fn encode(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut encoded = vec![tag];
    match content.len() {
        length @ 0..=127 => encoded.push(length as u8),
        length @ 128..=255 => encoded.extend([0x81, length as u8]),
        length => {
            encoded.push(0x82);
            encoded.extend((length as u16).to_be_bytes());
        }
    }
    encoded.extend(content);
    encoded
}

pub(crate) fn encode_oid(oid: &[u32]) -> Vec<u8> {
    let mut content = vec![(oid[0] * 40 + oid[1]) as u8];
    for &part in &oid[2..] {
        let mut groups = vec![(part & 0x7F) as u8];
        let mut rest = part >> 7;
        while rest > 0 {
            groups.push((rest & 0x7F) as u8 | 0x80);
            rest >>= 7;
        }
        content.extend(groups.iter().rev());
    }
    encode(TAG_OID, &content)
}

fn get_request(community: &str, oids: &[&[u32]]) -> Vec<u8> {
    let bindings: Vec<u8> = oids
        .iter()
        .flat_map(|oid| {
            let mut binding = encode_oid(oid);
            binding.extend(encode(TAG_NULL, &[]));
            encode(TAG_SEQUENCE, &binding)
        })
        .collect();
    let mut pdu = encode(TAG_INTEGER, &[REQUEST_ID]);
    pdu.extend(encode(TAG_INTEGER, &[0]));
    pdu.extend(encode(TAG_INTEGER, &[0]));
    pdu.extend(encode(TAG_SEQUENCE, &bindings));
    let mut message = encode(TAG_INTEGER, &[VERSION_2C]);
    message.extend(encode(TAG_OCTET_STRING, community.as_bytes()));
    message.extend(encode(TAG_GET_REQUEST, &pdu));
    encode(TAG_SEQUENCE, &message)
}

/// Reads one tag, length and value. Returns them and what follows.
pub(crate) fn read_element(data: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, rest) = data.split_first()?;
    let (&first, rest) = rest.split_first()?;
    let (length, rest) = match first {
        0..=127 => (usize::from(first), rest),
        0x81 => {
            let (&length, rest) = rest.split_first()?;
            (usize::from(length), rest)
        }
        0x82 => {
            let bytes = rest.get(..2)?;
            (
                usize::from(u16::from_be_bytes([bytes[0], bytes[1]])),
                &rest[2..],
            )
        }
        _ => return None,
    };
    let value = rest.get(..length)?;
    Some((tag, value, &rest[length..]))
}

pub(crate) fn read_unsigned(value: &[u8]) -> Option<u64> {
    if value.is_empty() || value.len() > 8 {
        return None;
    }
    Some(
        value
            .iter()
            .fold(0, |total, byte| (total << 8) | u64::from(*byte)),
    )
}

/// Replaces control characters and limits the length. Empty text becomes `None`.
pub(crate) fn clean_text(value: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(value);
    let cleaned: String = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_TEXT_CHARS)
        .collect();
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_owned())
}

/// The variable bindings of a response to our request.
fn response_bindings(packet: &[u8]) -> Option<&[u8]> {
    let (TAG_SEQUENCE, message, _) = read_element(packet)? else {
        return None;
    };
    let (TAG_INTEGER, _, rest) = read_element(message)? else {
        return None;
    };
    let (TAG_OCTET_STRING, _, rest) = read_element(rest)? else {
        return None;
    };
    let (TAG_RESPONSE, pdu, _) = read_element(rest)? else {
        return None;
    };
    let (TAG_INTEGER, request_id, rest) = read_element(pdu)? else {
        return None;
    };
    if request_id != [REQUEST_ID] {
        return None;
    }
    let (_, _, rest) = read_element(rest)?;
    let (_, _, rest) = read_element(rest)?;
    let (TAG_SEQUENCE, bindings, _) = read_element(rest)? else {
        return None;
    };
    Some(bindings)
}

fn parse_response(packet: &[u8]) -> Option<SnmpInfo> {
    let mut bindings = response_bindings(packet)?;
    let mut info = SnmpInfo::default();
    while let Some((TAG_SEQUENCE, binding, next)) = read_element(bindings) {
        bindings = next;
        let Some((TAG_OID, oid, after_oid)) = read_element(binding) else {
            continue;
        };
        let Some((kind, value, _)) = read_element(after_oid) else {
            continue;
        };
        apply_binding(&mut info, oid, kind, value);
    }
    (info != SnmpInfo::default()).then_some(info)
}

fn apply_binding(info: &mut SnmpInfo, oid: &[u8], kind: u8, value: &[u8]) {
    let matches = |wanted: &[u32]| encode_oid(wanted).get(2..) == Some(oid);
    let text = || {
        (kind == TAG_OCTET_STRING)
            .then(|| clean_text(value))
            .flatten()
    };
    if matches(SYS_DESCR) {
        info.description = text();
    } else if matches(SYS_NAME) {
        info.name = text();
    } else if matches(SYS_LOCATION) {
        info.location = text();
    } else if matches(ENTITY_MODEL) {
        info.model = text();
    } else if matches(ENTITY_SERIAL) {
        info.serial = text();
    } else if matches(SYS_UPTIME) && kind == TAG_TIMETICKS {
        info.uptime_seconds = read_unsigned(value).map(|ticks| ticks / TICKS_PER_SECOND);
    } else if matches(IF_NUMBER) && kind == TAG_INTEGER {
        info.interface_count = read_unsigned(value);
    }
}

async fn query_address(
    address: SocketAddrV4,
    community: &SnmpCommunity,
    wait: Duration,
) -> Option<SnmpInfo> {
    let _permit = socket_permit().await?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    socket.connect(SocketAddr::V4(address)).await.ok()?;
    socket
        .send(&get_request(&community.0, &QUERIED))
        .await
        .ok()?;
    let mut buffer = [0; 4096];
    let length = timeout(wait, socket.recv(&mut buffer)).await.ok()?.ok()?;
    parse_response(&buffer[..length])
}

/// Asks `ip` about itself. `None` when SNMP is off there or the community is wrong.
pub async fn query_device(
    ip: Ipv4Addr,
    community: &SnmpCommunity,
    wait: Duration,
) -> Option<SnmpInfo> {
    query_address(SocketAddrV4::new(ip, SNMP_PORT), community, wait).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(bindings: &[(&[u32], u8, &[u8])], request_id: u8) -> Vec<u8> {
        let list: Vec<u8> = bindings
            .iter()
            .flat_map(|(oid, kind, value)| {
                let mut binding = encode_oid(oid);
                binding.extend(encode(*kind, value));
                encode(TAG_SEQUENCE, &binding)
            })
            .collect();
        let mut pdu = encode(TAG_INTEGER, &[request_id]);
        pdu.extend(encode(TAG_INTEGER, &[0]));
        pdu.extend(encode(TAG_INTEGER, &[0]));
        pdu.extend(encode(TAG_SEQUENCE, &list));
        let mut message = encode(TAG_INTEGER, &[VERSION_2C]);
        message.extend(encode(TAG_OCTET_STRING, b"public"));
        message.extend(encode(TAG_RESPONSE, &pdu));
        encode(TAG_SEQUENCE, &message)
    }

    #[test]
    fn encodes_a_known_oid_and_long_lengths() {
        assert_eq!(
            encode_oid(SYS_DESCR),
            [0x06, 0x08, 0x2B, 6, 1, 2, 1, 1, 1, 0]
        );
        assert_eq!(&encode_oid(&[1, 3, 300])[2..], [0x2B, 0x82, 0x2C]);
        assert_eq!(&encode(TAG_OCTET_STRING, &[0; 200])[..3], [0x04, 0x81, 200]);
        assert_eq!(
            &encode(TAG_OCTET_STRING, &[0; 300])[..4],
            [0x04, 0x82, 1, 44]
        );
    }

    #[test]
    fn a_request_decodes_back_to_the_same_shape() {
        let request = get_request("secret", &QUERIED);
        let (tag, message, rest) = read_element(&request).unwrap();
        assert_eq!((tag, rest.len()), (TAG_SEQUENCE, 0));
        let (_, version, rest) = read_element(message).unwrap();
        assert_eq!(version, [VERSION_2C]);
        let (_, community, rest) = read_element(rest).unwrap();
        assert_eq!(community, b"secret");
        assert_eq!(read_element(rest).unwrap().0, TAG_GET_REQUEST);
    }

    #[test]
    fn reads_every_field() {
        let packet = response(
            &[
                (SYS_DESCR, TAG_OCTET_STRING, b"Switch OS 1.2\r\n"),
                (SYS_UPTIME, TAG_TIMETICKS, &[0x01, 0x00, 0x00]),
                (SYS_NAME, TAG_OCTET_STRING, b"core-switch"),
                (IF_NUMBER, TAG_INTEGER, &[24]),
                (ENTITY_SERIAL, TAG_OCTET_STRING, b"SN1"),
                (ENTITY_MODEL, TAG_OCTET_STRING, b"S-24"),
            ],
            REQUEST_ID,
        );
        let info = parse_response(&packet).unwrap();
        assert_eq!(info.description.as_deref(), Some("Switch OS 1.2"));
        assert_eq!(info.uptime_seconds, Some(655));
        assert_eq!(info.name.as_deref(), Some("core-switch"));
        assert_eq!(info.interface_count, Some(24));
        assert_eq!(info.serial.as_deref(), Some("SN1"));
        assert_eq!(info.model.as_deref(), Some("S-24"));
    }

    #[test]
    fn missing_values_and_other_requests_give_nothing() {
        let no_such = response(&[(ENTITY_MODEL, 0x80, &[])], REQUEST_ID);
        assert_eq!(parse_response(&no_such), None);
        let other = response(&[(SYS_NAME, TAG_OCTET_STRING, b"x")], REQUEST_ID + 1);
        assert_eq!(parse_response(&other), None);
    }

    #[test]
    fn survives_malformed_packets() {
        assert_eq!(parse_response(&[]), None);
        assert_eq!(parse_response(&[0x30, 0x82, 0xFF, 0xFF, 1]), None);
        let mut cut = response(&[(SYS_NAME, TAG_OCTET_STRING, b"name")], REQUEST_ID);
        cut.truncate(cut.len() - 2);
        assert_eq!(parse_response(&cut), None);
    }

    #[test]
    fn debug_output_hides_the_community() {
        let shown = format!("{:?}", SnmpCommunity("secret".into()));
        assert!(!shown.contains("secret"));
    }

    #[tokio::test]
    async fn asks_a_device_and_reads_the_answer() {
        let agent = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let SocketAddr::V4(address) = agent.local_addr().unwrap() else {
            panic!("bound to IPv4");
        };
        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            let (_, from) = agent.recv_from(&mut buffer).await.unwrap();
            let reply = response(&[(SYS_NAME, TAG_OCTET_STRING, b"agent")], REQUEST_ID);
            agent.send_to(&reply, from).await.unwrap();
        });
        let community = SnmpCommunity("public".into());
        let info = query_address(address, &community, Duration::from_secs(2))
            .await
            .unwrap();
        assert_eq!(info.name.as_deref(), Some("agent"));
    }

    #[tokio::test]
    async fn a_silent_device_gives_nothing() {
        let community = SnmpCommunity("public".into());
        let found = query_device(Ipv4Addr::LOCALHOST, &community, Duration::from_millis(50)).await;
        assert_eq!(found, None);
    }
}
