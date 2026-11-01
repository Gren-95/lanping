//! Reads the operating system's ARP / neighbor cache.
//!
//! Probing an address on the local network makes the OS resolve its MAC
//! address first, even when the host drops every packet afterwards. Reading
//! the cache after a sweep therefore finds hosts that ignore ping and TCP.

use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};

pub type NeighborTable = HashMap<Ipv4Addr, String>;

/// IPv6 addresses the OS has resolved, grouped by the MAC address that answered.
pub type Ipv6Table = HashMap<String, Vec<Ipv6Addr>>;

/// Returns IP to MAC (`AA:BB:CC:DD:EE:FF`) for every resolved neighbor.
pub async fn read_neighbor_table() -> NeighborTable {
    #[cfg(target_os = "linux")]
    {
        match tokio::fs::read_to_string("/proc/net/arp").await {
            Ok(text) => parse_proc_net_arp(&text),
            Err(_) => NeighborTable::new(),
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        run_arp_command().await
    }
}

#[cfg(not(target_os = "linux"))]
async fn run_arp_command() -> NeighborTable {
    use tokio::process::Command;
    let Ok(output) = Command::new("arp").arg("-a").output().await else {
        return NeighborTable::new();
    };
    parse_arp_command(&String::from_utf8_lossy(&output.stdout))
}

/// Reads the IPv6 neighbor cache. Empty on Windows and when the tool is missing.
pub async fn read_ipv6_neighbors() -> Ipv6Table {
    use tokio::process::Command;
    #[cfg(target_os = "linux")]
    let output = Command::new("ip")
        .args(["-6", "neigh", "show"])
        .output()
        .await;
    #[cfg(target_os = "macos")]
    let output = Command::new("ndp").arg("-an").output().await;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let output: std::io::Result<std::process::Output> = Err(std::io::ErrorKind::Unsupported.into());
    match output {
        Ok(output) => parse_ipv6_neighbors(&String::from_utf8_lossy(&output.stdout)),
        Err(_) => Ipv6Table::new(),
    }
}

/// Parses `ip -6 neigh show` (`fe80::1 dev wlan0 lladdr aa:bb:.. router REACHABLE`) and
/// `ndp -an` (`fe80::1%en0  aa:bb:cc:dd:ee:ff  en0  23h59m  R`). Skips entries without a MAC.
pub fn parse_ipv6_neighbors(text: &str) -> Ipv6Table {
    let mut table = Ipv6Table::new();
    for line in text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        let Some(address) = tokens.first().and_then(|raw| parse_ipv6(raw)) else {
            continue;
        };
        let mac = match tokens.iter().position(|token| *token == "lladdr") {
            Some(index) => tokens.get(index + 1).and_then(|raw| normalize_mac(raw)),
            None => tokens.get(1).and_then(|raw| normalize_mac(raw)),
        };
        if let Some(mac) = mac {
            let addresses = table.entry(mac).or_default();
            if !addresses.contains(&address) {
                addresses.push(address);
            }
        }
    }
    for addresses in table.values_mut() {
        addresses.sort();
    }
    table
}

/// Parses an address that may carry a zone, as in `fe80::1%en0`.
fn parse_ipv6(raw: &str) -> Option<Ipv6Addr> {
    raw.split('%').next()?.parse().ok()
}

/// Normalizes `aa-bb-cc-dd-ee-ff`, `a:b:c:d:e:f` and similar into
/// `AA:BB:CC:DD:EE:FF`. Returns `None` for incomplete or broadcast entries.
pub fn normalize_mac(raw: &str) -> Option<String> {
    let parts: Vec<&str> = raw.split([':', '-']).collect();
    if parts.len() != 6 {
        return None;
    }
    let mut bytes = [0u8; 6];
    for (slot, part) in bytes.iter_mut().zip(&parts) {
        if part.is_empty() || part.len() > 2 {
            return None;
        }
        *slot = u8::from_str_radix(part, 16).ok()?;
    }
    if bytes == [0; 6] || bytes == [0xff; 6] {
        return None;
    }
    Some(
        bytes
            .iter()
            .map(|byte| format!("{byte:02X}"))
            .collect::<Vec<_>>()
            .join(":"),
    )
}

/// Parses the Linux `/proc/net/arp` format. Skips incomplete entries.
pub fn parse_proc_net_arp(text: &str) -> NeighborTable {
    const FLAG_COMPLETE: u32 = 0x2;
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let columns: Vec<&str> = line.split_whitespace().collect();
            let [ip, _hw_type, flags, mac, ..] = columns.as_slice() else {
                return None;
            };
            let flags = u32::from_str_radix(flags.trim_start_matches("0x"), 16).ok()?;
            if flags & FLAG_COMPLETE == 0 {
                return None;
            }
            Some((ip.parse().ok()?, normalize_mac(mac)?))
        })
        .collect()
}

/// Parses `arp -a` output from macOS (`? (10.0.0.1) at a:b:c:d:e:f on en0`)
/// and Windows (`  10.0.0.1   aa-bb-cc-dd-ee-ff   dynamic`).
pub fn parse_arp_command(text: &str) -> NeighborTable {
    text.lines().filter_map(parse_arp_line).collect()
}

fn parse_arp_line(line: &str) -> Option<(Ipv4Addr, String)> {
    let tokens: Vec<&str> = line.split_whitespace().collect();

    if let Some(at_index) = tokens.iter().position(|token| *token == "at") {
        let ip_token = tokens.get(at_index.checked_sub(1)?)?;
        let ip = ip_token.trim_matches(['(', ')']).parse().ok()?;
        let mac = normalize_mac(tokens.get(at_index + 1)?)?;
        return Some((ip, mac));
    }

    let ip = tokens.first()?.parse().ok()?;
    let mac = normalize_mac(tokens.get(1)?)?;
    Some((ip, mac))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_macs() {
        assert_eq!(
            normalize_mac("aa-bb-cc-dd-ee-0f").as_deref(),
            Some("AA:BB:CC:DD:EE:0F")
        );
        assert_eq!(
            normalize_mac("a:b:c:d:e:f").as_deref(),
            Some("0A:0B:0C:0D:0E:0F")
        );
    }

    #[test]
    fn rejects_unusable_macs() {
        assert_eq!(normalize_mac("00:00:00:00:00:00"), None);
        assert_eq!(normalize_mac("ff-ff-ff-ff-ff-ff"), None);
        assert_eq!(normalize_mac("(incomplete)"), None);
        assert_eq!(normalize_mac("aa:bb:cc"), None);
        assert_eq!(normalize_mac("zz:bb:cc:dd:ee:ff"), None);
    }

    #[test]
    fn parses_linux_and_macos_ipv6_neighbors() {
        let linux = "\
fe80::1 dev wlan0 lladdr aa:bb:cc:dd:ee:01 router REACHABLE
2001:db8::5 dev wlan0 lladdr aa:bb:cc:dd:ee:01 STALE
fe80::9 dev wlan0  FAILED
";
        let table = parse_ipv6_neighbors(linux);
        assert_eq!(table.len(), 1);
        let addresses = &table["AA:BB:CC:DD:EE:01"];
        assert_eq!(addresses.len(), 2);
        assert!(addresses.contains(&"fe80::1".parse().unwrap()));

        let macos = "\
Neighbor                        Linklayer Address  Netif Expire    St Flgs Prbs
fe80::1%en0                     a:b:c:d:e:f        en0 23h59m59s  R
fe80::2%en0                     (incomplete)       en0 expired    N
";
        let table = parse_ipv6_neighbors(macos);
        assert_eq!(table.len(), 1);
        assert_eq!(
            table["0A:0B:0C:0D:0E:0F"],
            ["fe80::1".parse::<Ipv6Addr>().unwrap()]
        );
    }

    #[test]
    fn parses_linux_table() {
        let text = "\
IP address       HW type     Flags       HW address            Mask     Device
192.168.0.1      0x1         0x2         aa:bb:cc:dd:ee:01     *        wlan0
192.168.0.9      0x1         0x0         00:00:00:00:00:00     *        wlan0
192.168.0.20     0x1         0x2         aa:bb:cc:dd:ee:20     *        wlan0
";
        let table = parse_proc_net_arp(text);
        assert_eq!(table.len(), 2);
        assert_eq!(
            table
                .get(&"192.168.0.1".parse().unwrap())
                .map(String::as_str),
            Some("AA:BB:CC:DD:EE:01")
        );
    }

    #[test]
    fn parses_macos_output() {
        let text = "\
? (192.168.0.1) at a0:b:c:d:e:1 on en0 ifscope [ethernet]
? (192.168.0.77) at (incomplete) on en0 ifscope [ethernet]
? (224.0.0.251) at 1:0:5e:0:0:fb on en0 ifscope permanent [ethernet]
";
        let table = parse_arp_command(text);
        assert_eq!(
            table
                .get(&"192.168.0.1".parse().unwrap())
                .map(String::as_str),
            Some("A0:0B:0C:0D:0E:01")
        );
        assert!(!table.contains_key(&"192.168.0.77".parse().unwrap()));
    }

    #[test]
    fn parses_windows_output() {
        let text = "\
Interface: 192.168.0.5 --- 0x9
  Internet Address      Physical Address      Type
  192.168.0.1           aa-bb-cc-dd-ee-01     dynamic
  255.255.255.255       ff-ff-ff-ff-ff-ff     static
";
        let table = parse_arp_command(text);
        assert_eq!(table.len(), 1);
        assert_eq!(
            table
                .get(&"192.168.0.1".parse().unwrap())
                .map(String::as_str),
            Some("AA:BB:CC:DD:EE:01")
        );
    }
}
