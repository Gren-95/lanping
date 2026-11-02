//! NetBIOS node status query (UDP 137): asks a host for its computer name.

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::time::timeout;

const NETBIOS_PORT: u16 = 137;
const NAME_ENTRY_LENGTH: usize = 18;
/// Header (12), encoded name (34), type, class, TTL and data length (10).
const NAME_COUNT_OFFSET: usize = 56;
const WORKSTATION_SUFFIX: u8 = 0x00;
const GROUP_NAME_FLAG: u8 = 0x80;

/// A node status request for the wildcard name `*`.
fn status_request() -> Vec<u8> {
    let mut packet = vec![0x4C, 0x50, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0x20, b'C', b'K'];
    packet.extend(std::iter::repeat_n(b'A', 30));
    packet.extend([0, 0, 0x21, 0, 1]);
    packet
}

/// The unique workstation name in a node status response.
fn parse_status_response(packet: &[u8]) -> Option<String> {
    let count = usize::from(*packet.get(NAME_COUNT_OFFSET)?);
    let names = packet.get(NAME_COUNT_OFFSET + 1..)?;
    names
        .as_chunks::<NAME_ENTRY_LENGTH>()
        .0
        .iter()
        .take(count)
        .find(|entry| entry[15] == WORKSTATION_SUFFIX && entry[16] & GROUP_NAME_FLAG == 0)
        .map(|entry| String::from_utf8_lossy(&entry[..15]).trim().to_owned())
        .filter(|name| !name.is_empty())
}

/// The NetBIOS name of `ip`, or `None` when it does not answer in time.
pub async fn query_name(ip: Ipv4Addr, wait: Duration) -> Option<String> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await.ok()?;
    socket.connect((ip, NETBIOS_PORT)).await.ok()?;
    socket.send(&status_request()).await.ok()?;
    let mut buffer = [0; 1024];
    let length = timeout(wait, socket.recv(&mut buffer)).await.ok()?.ok()?;
    parse_status_response(&buffer[..length])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response(entries: &[(&str, u8, u8)]) -> Vec<u8> {
        let mut packet = vec![0; NAME_COUNT_OFFSET];
        packet.push(entries.len() as u8);
        for (name, suffix, flags) in entries {
            packet.extend(format!("{name:<15}").into_bytes());
            packet.extend([*suffix, *flags, 0]);
        }
        packet
    }

    #[test]
    fn request_has_the_standard_length_and_encodes_the_wildcard() {
        let request = status_request();
        assert_eq!(request.len(), 50);
        assert_eq!(&request[13..15], b"CK");
    }

    #[test]
    fn picks_the_unique_workstation_name() {
        let packet = response(&[
            ("WORKGROUP", 0x00, GROUP_NAME_FLAG),
            ("NAS", 0x20, 0),
            ("NAS", 0x00, 0),
        ]);
        assert_eq!(parse_status_response(&packet).as_deref(), Some("NAS"));
    }

    #[test]
    fn ignores_short_or_nameless_packets() {
        assert_eq!(parse_status_response(&[0; 10]), None);
        let only_group = response(&[("WORKGROUP", 0x00, GROUP_NAME_FLAG)]);
        assert_eq!(parse_status_response(&only_group), None);
        let mut truncated = response(&[("NAS", 0x00, 0)]);
        truncated.truncate(NAME_COUNT_OFFSET + 5);
        assert_eq!(parse_status_response(&truncated), None);
    }

    #[tokio::test]
    async fn a_silent_host_gives_no_name() {
        let name = query_name(Ipv4Addr::LOCALHOST, Duration::from_millis(50)).await;
        assert_eq!(name, None);
    }
}
