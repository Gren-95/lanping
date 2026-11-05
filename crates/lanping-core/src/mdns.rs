//! Direct mDNS questions (UDP 5353) to one host: its `.local` name and its service types.

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;

use crate::socket_budget::socket_permit;

pub(crate) const MDNS_PORT: u16 = 5353;
pub(crate) const SERVICE_LIST_NAME: &str = "_services._dns-sd._udp.local";
pub(crate) const LOCAL_SUFFIX: &str = ".local";
pub(crate) const RECORD_PTR: u16 = 12;
const CLASS_IN: u16 = 1;
pub(crate) const HEADER_LENGTH: usize = 12;
const MAX_NAME_JUMPS: usize = 16;

pub(crate) fn query_packet(name: &str) -> Vec<u8> {
    let mut packet = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in name.split('.') {
        packet.push(label.len() as u8);
        packet.extend(label.as_bytes());
    }
    packet.push(0);
    packet.extend(RECORD_PTR.to_be_bytes());
    packet.extend(CLASS_IN.to_be_bytes());
    packet
}

fn reverse_name(ip: Ipv4Addr) -> String {
    let [a, b, c, d] = ip.octets();
    format!("{d}.{c}.{b}.{a}.in-addr.arpa")
}

/// Reads a possibly compressed DNS name. Returns it and the offset just after it.
pub(crate) fn read_name(packet: &[u8], start: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut position = start;
    let mut end = None;
    let mut jumps = 0;
    loop {
        let length = *packet.get(position)?;
        match length {
            0 => {
                return Some((labels.join("."), end.unwrap_or(position + 1)));
            }
            0xC0..=0xFF => {
                jumps += 1;
                if jumps > MAX_NAME_JUMPS {
                    return None;
                }
                let low = *packet.get(position + 1)?;
                end.get_or_insert(position + 2);
                position = (usize::from(length & 0x3F) << 8) | usize::from(low);
            }
            1..=63 => {
                let label = packet.get(position + 1..position + 1 + usize::from(length))?;
                labels.push(String::from_utf8_lossy(label).into_owned());
                position += 1 + usize::from(length);
            }
            _ => return None,
        }
    }
}

pub(crate) fn read_u16(packet: &[u8], offset: usize) -> Option<u16> {
    let bytes = packet.get(offset..offset + 2)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
}

/// The targets of every PTR answer in a response. Stops at the first malformed record.
fn ptr_answers(packet: &[u8]) -> Vec<String> {
    let mut answers = Vec::new();
    let (Some(questions), Some(records)) = (read_u16(packet, 4), read_u16(packet, 6)) else {
        return answers;
    };
    let mut position = HEADER_LENGTH;
    for _ in 0..questions {
        let Some((_, after)) = read_name(packet, position) else {
            return answers;
        };
        position = after + 4;
    }
    for _ in 0..records {
        let Some((_, after)) = read_name(packet, position) else {
            return answers;
        };
        let (Some(kind), Some(length)) = (read_u16(packet, after), read_u16(packet, after + 8))
        else {
            return answers;
        };
        let data = after + 10;
        if kind == RECORD_PTR
            && let Some((target, _)) = read_name(packet, data)
        {
            answers.push(target);
        }
        position = data + usize::from(length);
    }
    answers
}

/// `_http._tcp.local` becomes `http`.
fn service_type(name: &str) -> Option<String> {
    let bare = name.strip_suffix(LOCAL_SUFFIX)?;
    let (service, _protocol) = bare.split_once('.')?;
    let service = service.strip_prefix('_')?;
    (!service.is_empty()).then(|| service.to_owned())
}

async fn ask(ip: Ipv4Addr, name: &str, wait: Duration) -> Vec<String> {
    let Some(_permit) = socket_permit().await else {
        return Vec::new();
    };
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
        return Vec::new();
    };
    if socket.connect((ip, MDNS_PORT)).await.is_err()
        || socket.send(&query_packet(name)).await.is_err()
    {
        return Vec::new();
    }
    let mut buffer = [0; 2048];
    match timeout(wait, socket.recv(&mut buffer)).await {
        Ok(Ok(length)) => ptr_answers(&buffer[..length]),
        _ => Vec::new(),
    }
}

/// The `.local` name of `ip`, without the suffix.
pub async fn query_name(ip: Ipv4Addr, wait: Duration) -> Option<String> {
    let answers = ask(ip, &reverse_name(ip), wait).await;
    let name = answers.first()?;
    let bare = name.strip_suffix(LOCAL_SUFFIX).unwrap_or(name);
    (!bare.is_empty()).then(|| bare.to_owned())
}

/// The service types `ip` advertises, such as `http` or `ipp`, sorted.
pub async fn query_services(ip: Ipv4Addr, wait: Duration) -> Vec<String> {
    let mut services: Vec<String> = ask(ip, SERVICE_LIST_NAME, wait)
        .await
        .iter()
        .filter_map(|name| service_type(name))
        .collect();
    services.sort();
    services.dedup();
    services
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A response with one PTR answer whose owner name points back at the question.
    fn response(question: &str, target: &str) -> Vec<u8> {
        let mut packet = query_packet(question);
        packet[2] = 0x84;
        packet[7] = 1;
        packet.extend([0xC0, HEADER_LENGTH as u8]);
        packet.extend(RECORD_PTR.to_be_bytes());
        packet.extend(CLASS_IN.to_be_bytes());
        packet.extend([0, 0, 0, 120]);
        let mut data = Vec::new();
        for label in target.split('.') {
            data.push(label.len() as u8);
            data.extend(label.as_bytes());
        }
        data.push(0);
        packet.extend((data.len() as u16).to_be_bytes());
        packet.extend(data);
        packet
    }

    #[test]
    fn builds_the_reverse_lookup_name() {
        assert_eq!(
            reverse_name(Ipv4Addr::new(192, 168, 1, 7)),
            "7.1.168.192.in-addr.arpa"
        );
    }

    #[test]
    fn reads_ptr_answers() {
        let packet = response("7.1.168.192.in-addr.arpa", "printer.local");
        assert_eq!(ptr_answers(&packet), ["printer.local"]);
    }

    #[test]
    fn follows_compression_and_rejects_loops() {
        let packet = [3, b'n', b'a', b's', 0, 0xC0, 0];
        assert_eq!(read_name(&packet, 5), Some(("nas".to_owned(), 7)));
        let looping = [0xC0, 0];
        assert_eq!(read_name(&looping, 0), None);
    }

    #[test]
    fn survives_malformed_packets() {
        assert!(ptr_answers(&[]).is_empty());
        assert!(ptr_answers(&[0, 0, 0, 0, 0, 9, 0, 9, 0, 0, 0, 0, 200]).is_empty());
        let mut cut = response("a.local", "b.local");
        cut.truncate(cut.len() - 3);
        assert!(ptr_answers(&cut).is_empty());
    }

    #[test]
    fn extracts_service_types() {
        assert_eq!(service_type("_ipp._tcp.local").as_deref(), Some("ipp"));
        assert_eq!(service_type("_http._tcp").as_deref(), None);
        assert_eq!(service_type("printer.local").as_deref(), None);
    }
}
