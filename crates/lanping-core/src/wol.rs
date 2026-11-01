//! Wake-on-LAN magic packets.

use std::io;
use std::net::{Ipv4Addr, UdpSocket};

const WOL_PORT: u16 = 9;
const PACKET_LENGTH: usize = 6 + 16 * 6;

/// Parses `AA:BB:CC:DD:EE:FF` (colon or dash separated).
pub fn parse_mac(text: &str) -> Option<[u8; 6]> {
    let mut bytes = [0u8; 6];
    let mut parts = text.split([':', '-']);
    for slot in &mut bytes {
        *slot = u8::from_str_radix(parts.next()?, 16).ok()?;
    }
    parts.next().is_none().then_some(bytes)
}

/// Six `0xFF` bytes followed by the MAC repeated sixteen times.
pub fn magic_packet(mac: [u8; 6]) -> [u8; PACKET_LENGTH] {
    let mut packet = [0xFF; PACKET_LENGTH];
    for repeat in packet[6..].chunks_exact_mut(6) {
        repeat.copy_from_slice(&mac);
    }
    packet
}

/// Broadcasts a magic packet for `mac` on the local network.
pub fn send_wake_packet(mac: &str) -> io::Result<()> {
    let bytes = parse_mac(mac)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid MAC address"))?;
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))?;
    socket.set_broadcast(true)?;
    socket.send_to(&magic_packet(bytes), (Ipv4Addr::BROADCAST, WOL_PORT))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_and_invalid_macs() {
        assert_eq!(parse_mac("AA:bb:CC:dd:EE:01"), Some([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x01]));
        assert_eq!(parse_mac("aa-bb-cc-dd-ee-01"), Some([0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0x01]));
        assert_eq!(parse_mac("aa:bb:cc"), None);
        assert_eq!(parse_mac("aa:bb:cc:dd:ee:ff:00"), None);
        assert_eq!(parse_mac("zz:bb:cc:dd:ee:ff"), None);
    }

    #[test]
    fn builds_magic_packet() {
        let mac = [1, 2, 3, 4, 5, 6];
        let packet = magic_packet(mac);
        assert_eq!(packet.len(), 102);
        assert!(packet[..6].iter().all(|byte| *byte == 0xFF));
        assert!(packet[6..].chunks(6).all(|chunk| chunk == mac));
    }

    #[test]
    fn rejects_bad_mac_before_sending() {
        assert!(send_wake_packet("nope").is_err());
    }
}
