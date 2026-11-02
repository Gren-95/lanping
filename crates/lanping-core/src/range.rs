use std::net::Ipv4Addr;

use ipnet::Ipv4Net;

/// Largest range one scan may cover (a /16).
pub const MAX_TARGETS: usize = 65_536;

#[derive(Debug, PartialEq, Eq)]
pub enum RangeError {
    Empty,
    Invalid(String),
    TooLarge,
}

impl std::fmt::Display for RangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "Enter an IP, range or CIDR block"),
            Self::Invalid(text) => write!(f, "Cannot read target: {text}"),
            Self::TooLarge => write!(f, "Range is larger than {MAX_TARGETS} addresses"),
        }
    }
}

/// Parses a comma separated list of targets. Each entry is one of:
/// `192.168.0.5`, `192.168.0.1-192.168.0.50`, `192.168.0.1-50`, `192.168.0.0/24`.
pub fn parse_targets(text: &str) -> Result<Vec<Ipv4Addr>, RangeError> {
    let entries: Vec<&str> = text
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .collect();
    if entries.is_empty() {
        return Err(RangeError::Empty);
    }

    let mut addresses = Vec::new();
    for entry in entries {
        addresses.extend(parse_entry(entry)?);
        if addresses.len() > MAX_TARGETS {
            return Err(RangeError::TooLarge);
        }
    }
    addresses.sort_unstable();
    addresses.dedup();
    Ok(addresses)
}

fn parse_entry(entry: &str) -> Result<Vec<Ipv4Addr>, RangeError> {
    let invalid = || RangeError::Invalid(entry.to_owned());

    if entry.contains('/') {
        let net: Ipv4Net = entry.parse().map_err(|_| invalid())?;
        if net.hosts().count() > MAX_TARGETS {
            return Err(RangeError::TooLarge);
        }
        return Ok(net.hosts().collect());
    }

    if let Some((start_text, end_text)) = entry.split_once('-') {
        let start: Ipv4Addr = start_text.trim().parse().map_err(|_| invalid())?;
        let end = parse_range_end(start, end_text.trim()).ok_or_else(invalid)?;
        let (first, last) = (u32::from(start), u32::from(end));
        if first > last {
            return Err(invalid());
        }
        if (last - first) as usize >= MAX_TARGETS {
            return Err(RangeError::TooLarge);
        }
        return Ok((first..=last).map(Ipv4Addr::from).collect());
    }

    let single: Ipv4Addr = entry.parse().map_err(|_| invalid())?;
    Ok(vec![single])
}

/// The end of a range is either a full address or only the last octet.
fn parse_range_end(start: Ipv4Addr, end_text: &str) -> Option<Ipv4Addr> {
    if let Ok(full) = end_text.parse::<Ipv4Addr>() {
        return Some(full);
    }
    let last_octet: u8 = end_text.parse().ok()?;
    let [a, b, c, _] = start.octets();
    Some(Ipv4Addr::new(a, b, c, last_octet))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkKind {
    /// A real LAN or Wi-Fi interface: the usual scan target.
    Physical,
    /// The online devices of a Tailscale network.
    Tailnet,
    /// Container bridge, VPN tunnel or VM switch.
    Virtual,
}

/// A scan target offered to the user: an interface's network, or the
/// device list of a Tailscale network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalNetwork {
    pub interface: String,
    /// Text for the target field; understood by [`parse_targets`].
    pub target: String,
    pub kind: NetworkKind,
    pub host_count: u32,
}

impl std::fmt::Display for LocalNetwork {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            NetworkKind::Physical => write!(
                f,
                "{}  -  {}  ({} hosts)",
                self.interface, self.target, self.host_count
            ),
            NetworkKind::Virtual => write!(
                f,
                "{}  -  {}  ({} hosts, virtual)",
                self.interface, self.target, self.host_count
            ),
            NetworkKind::Tailnet => write!(
                f,
                "{}  -  Tailscale devices  ({} online)",
                self.interface, self.host_count
            ),
        }
    }
}

/// Private IPv4 networks of the machine's interfaces plus the online
/// Tailscale devices, recommended (physical) ones first. Single-address and
/// oversized networks are left out because they cannot be swept.
pub fn detect_local_networks() -> Vec<LocalNetwork> {
    let mut networks = interface_networks();
    let peers = crate::tailnet::online_peers();
    if !peers.is_empty() {
        networks.push(LocalNetwork {
            interface: "Tailscale".to_owned(),
            target: peers
                .iter()
                .map(Ipv4Addr::to_string)
                .collect::<Vec<_>>()
                .join(", "),
            kind: NetworkKind::Tailnet,
            host_count: peers.len() as u32,
        });
    }
    networks.sort_by_key(|network| match network.kind {
        NetworkKind::Physical => 0,
        NetworkKind::Tailnet => 1,
        NetworkKind::Virtual => 2,
    });
    networks
}

fn interface_networks() -> Vec<LocalNetwork> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    interfaces
        .into_iter()
        .filter(|interface| !interface.is_loopback())
        .filter_map(|interface| {
            let if_addrs::IfAddr::V4(v4) = &interface.addr else {
                return None;
            };
            if !v4.ip.is_private() {
                return None;
            }
            let prefix = u32::from(v4.netmask).count_ones() as u8;
            let net = Ipv4Net::new(v4.ip, prefix).ok()?.trunc();
            let host_count = net.hosts().count();
            if !(2..=MAX_TARGETS).contains(&host_count) {
                return None;
            }
            let kind = if is_virtual_interface(&interface.name) {
                NetworkKind::Virtual
            } else {
                NetworkKind::Physical
            };
            Some(LocalNetwork {
                interface: interface.name,
                target: net.to_string(),
                kind,
                host_count: host_count as u32,
            })
        })
        .collect()
}

/// Container bridges, VPN tunnels and VM switches.
fn is_virtual_interface(name: &str) -> bool {
    const VIRTUAL_PREFIXES: [&str; 9] = [
        "docker",
        "br-",
        "veth",
        "virbr",
        "tailscale",
        "tun",
        "tap",
        "zt",
        "vEthernet",
    ];
    VIRTUAL_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_physical_and_virtual_networks() {
        let lan = LocalNetwork {
            interface: "eth0".into(),
            target: "192.168.1.0/24".into(),
            kind: NetworkKind::Physical,
            host_count: 254,
        };
        assert_eq!(lan.to_string(), "eth0  -  192.168.1.0/24  (254 hosts)");
        let bridge = LocalNetwork {
            kind: NetworkKind::Virtual,
            ..lan.clone()
        };
        assert!(bridge.to_string().ends_with("virtual)"));
        let tailnet = LocalNetwork {
            interface: "Tailscale".into(),
            target: "100.64.0.1, 100.64.0.2".into(),
            kind: NetworkKind::Tailnet,
            host_count: 2,
        };
        assert_eq!(
            tailnet.to_string(),
            "Tailscale  -  Tailscale devices  (2 online)"
        );
        assert_eq!(parse_targets(&tailnet.target).unwrap().len(), 2);
    }

    #[test]
    fn recognizes_virtual_interfaces() {
        assert!(is_virtual_interface("docker0"));
        assert!(is_virtual_interface("br-bb602f6ce717"));
        assert!(is_virtual_interface("tailscale0"));
        assert!(!is_virtual_interface("wlan0"));
        assert!(!is_virtual_interface("eth0"));
    }

    fn ip(text: &str) -> Ipv4Addr {
        text.parse().unwrap()
    }

    #[test]
    fn parses_single_address() {
        assert_eq!(parse_targets("10.0.0.5").unwrap(), vec![ip("10.0.0.5")]);
    }

    #[test]
    fn parses_cidr_without_network_and_broadcast() {
        let targets = parse_targets("192.168.1.0/24").unwrap();
        assert_eq!(targets.len(), 254);
        assert_eq!(targets.first(), Some(&ip("192.168.1.1")));
        assert_eq!(targets.last(), Some(&ip("192.168.1.254")));
    }

    #[test]
    fn parses_point_to_point_block() {
        let targets = parse_targets("10.0.0.0/31").unwrap();
        assert_eq!(targets, vec![ip("10.0.0.0"), ip("10.0.0.1")]);
    }

    #[test]
    fn parses_short_range() {
        let targets = parse_targets("192.168.1.10-12").unwrap();
        assert_eq!(
            targets,
            vec![ip("192.168.1.10"), ip("192.168.1.11"), ip("192.168.1.12")]
        );
    }

    #[test]
    fn parses_full_range_across_octets() {
        let targets = parse_targets("10.0.0.254-10.0.1.1").unwrap();
        assert_eq!(targets.len(), 4);
    }

    #[test]
    fn merges_and_dedups_entries() {
        let targets = parse_targets("10.0.0.1, 10.0.0.1-2").unwrap();
        assert_eq!(targets, vec![ip("10.0.0.1"), ip("10.0.0.2")]);
    }

    #[test]
    fn rejects_empty_input() {
        assert_eq!(parse_targets("  ,, "), Err(RangeError::Empty));
    }

    #[test]
    fn rejects_garbage() {
        assert!(matches!(
            parse_targets("not-an-ip"),
            Err(RangeError::Invalid(_))
        ));
        assert!(matches!(
            parse_targets("10.0.0.0/33"),
            Err(RangeError::Invalid(_))
        ));
        assert!(matches!(
            parse_targets("10.0.0.9-10.0.0.1"),
            Err(RangeError::Invalid(_))
        ));
        assert!(matches!(
            parse_targets("10.0.0.1-300"),
            Err(RangeError::Invalid(_))
        ));
    }

    #[test]
    fn rejects_oversized_range() {
        assert_eq!(parse_targets("10.0.0.0/8"), Err(RangeError::TooLarge));
    }
}
