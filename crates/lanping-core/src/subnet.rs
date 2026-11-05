//! Subnet calculator: what an address with a prefix length or netmask covers.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ipnet::{IpNet, Ipv4Net, Ipv6Net};

use crate::range::MAX_TARGETS;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubnetError {
    Empty,
    Invalid,
}

impl fmt::Display for SubnetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("Enter an address with a prefix length or a netmask"),
            Self::Invalid => f.write_str("Not a valid address with a prefix length or netmask"),
        }
    }
}

/// One line of the result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubnetRow {
    pub label: &'static str,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Subnet {
    /// The network in CIDR form, such as `192.168.1.0/24`.
    pub network: String,
    pub rows: Vec<SubnetRow>,
    /// The network in CIDR form when a LAN scan can cover it.
    pub scan_target: Option<String>,
}

/// Reads `192.168.1.10/24`, `192.168.1.10 255.255.255.0`, `fe80::1/64` or a bare address.
pub fn describe(input: &str) -> Result<Subnet, SubnetError> {
    let net = parse_net(input)?;
    Ok(match net {
        IpNet::V4(net) => describe_v4(net),
        IpNet::V6(net) => describe_v6(net),
    })
}

fn parse_net(input: &str) -> Result<IpNet, SubnetError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(SubnetError::Empty);
    }
    if let Ok(net) = input.parse::<IpNet>() {
        return Ok(net);
    }
    if let Ok(address) = input.parse::<IpAddr>() {
        return Ok(IpNet::from(address));
    }
    let mut parts = input.split_whitespace();
    let (Some(address), Some(mask), None) = (parts.next(), parts.next(), parts.next()) else {
        return Err(SubnetError::Invalid);
    };
    let address: Ipv4Addr = address.parse().map_err(|_| SubnetError::Invalid)?;
    let mask: Ipv4Addr = mask.parse().map_err(|_| SubnetError::Invalid)?;
    Ipv4Net::with_netmask(address, mask)
        .map(IpNet::V4)
        .map_err(|_| SubnetError::Invalid)
}

fn row(label: &'static str, value: impl ToString) -> SubnetRow {
    SubnetRow {
        label,
        value: value.to_string(),
    }
}

fn describe_v4(net: Ipv4Net) -> Subnet {
    let prefix = net.prefix_len();
    let total = 1_u64 << (32 - prefix);
    let (first, last, usable) = match prefix {
        32 => (net.addr(), net.addr(), 1),
        31 => (net.network(), net.broadcast(), 2),
        _ => (
            Ipv4Addr::from(u32::from(net.network()) + 1),
            Ipv4Addr::from(u32::from(net.broadcast()) - 1),
            total - 2,
        ),
    };
    let network = net.trunc();
    Subnet {
        network: network.to_string(),
        rows: vec![
            row("Address", net.addr()),
            row("Network", net.network()),
            row("Prefix length", format!("/{prefix}")),
            row("Netmask", net.netmask()),
            row("Wildcard mask", net.hostmask()),
            row("Broadcast", net.broadcast()),
            row("First host", first),
            row("Last host", last),
            row("Usable hosts", usable),
            row("Total addresses", total),
        ],
        scan_target: (usable <= MAX_TARGETS as u64).then(|| network.to_string()),
    }
}

fn describe_v6(net: Ipv6Net) -> Subnet {
    let prefix = net.prefix_len();
    let host_bits = 128 - prefix;
    let total = match host_bits {
        0 => "1".to_owned(),
        bits if bits < 64 => (1_u64 << bits).to_string(),
        bits => format!("2^{bits}"),
    };
    let last = Ipv6Addr::from(u128::from(net.network()) | u128::from(net.hostmask()));
    let network = net.trunc();
    Subnet {
        network: network.to_string(),
        rows: vec![
            row("Address", net.addr()),
            row("Network", net.network()),
            row("Prefix length", format!("/{prefix}")),
            row("Netmask", net.netmask()),
            row("First address", net.network()),
            row("Last address", last),
            row("Total addresses", total),
        ],
        scan_target: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn value<'a>(subnet: &'a Subnet, label: &str) -> &'a str {
        &subnet
            .rows
            .iter()
            .find(|row| row.label == label)
            .unwrap_or_else(|| panic!("no row {label}"))
            .value
    }

    #[test]
    fn describes_a_typical_lan() {
        let subnet = describe("192.168.1.10/24").unwrap();
        assert_eq!(subnet.network, "192.168.1.0/24");
        assert_eq!(value(&subnet, "Address"), "192.168.1.10");
        assert_eq!(value(&subnet, "Network"), "192.168.1.0");
        assert_eq!(value(&subnet, "Netmask"), "255.255.255.0");
        assert_eq!(value(&subnet, "Wildcard mask"), "0.0.0.255");
        assert_eq!(value(&subnet, "Broadcast"), "192.168.1.255");
        assert_eq!(value(&subnet, "First host"), "192.168.1.1");
        assert_eq!(value(&subnet, "Last host"), "192.168.1.254");
        assert_eq!(value(&subnet, "Usable hosts"), "254");
        assert_eq!(value(&subnet, "Total addresses"), "256");
        assert_eq!(subnet.scan_target.as_deref(), Some("192.168.1.0/24"));
    }

    #[test]
    fn reads_a_dotted_netmask() {
        let subnet = describe("10.1.2.3 255.255.0.0").unwrap();
        assert_eq!(subnet.network, "10.1.0.0/16");
        assert_eq!(value(&subnet, "Usable hosts"), "65534");
    }

    #[test]
    fn refuses_a_netmask_with_gaps() {
        assert_eq!(describe("10.0.0.1 255.0.255.0"), Err(SubnetError::Invalid));
    }

    #[test]
    fn point_to_point_and_single_host_networks() {
        let pair = describe("10.0.0.0/31").unwrap();
        assert_eq!(value(&pair, "Usable hosts"), "2");
        assert_eq!(value(&pair, "First host"), "10.0.0.0");
        assert_eq!(value(&pair, "Last host"), "10.0.0.1");
        let single = describe("10.0.0.7/32").unwrap();
        assert_eq!(value(&single, "Usable hosts"), "1");
        assert_eq!(value(&single, "First host"), "10.0.0.7");
    }

    #[test]
    fn a_bare_address_is_a_single_host() {
        let subnet = describe("10.0.0.7").unwrap();
        assert_eq!(subnet.network, "10.0.0.7/32");
    }

    #[test]
    fn a_range_too_large_to_scan_has_no_scan_target() {
        assert_eq!(describe("10.0.0.0/8").unwrap().scan_target, None);
        assert!(describe("10.0.0.0/16").unwrap().scan_target.is_some());
    }

    #[test]
    fn the_whole_address_space_does_not_overflow() {
        let subnet = describe("0.0.0.0/0").unwrap();
        assert_eq!(value(&subnet, "Total addresses"), "4294967296");
        assert_eq!(value(&subnet, "Usable hosts"), "4294967294");
    }

    #[test]
    fn describes_ipv6() {
        let subnet = describe("2001:db8::1/64").unwrap();
        assert_eq!(subnet.network, "2001:db8::/64");
        assert_eq!(
            value(&subnet, "Last address"),
            "2001:db8::ffff:ffff:ffff:ffff"
        );
        assert_eq!(value(&subnet, "Total addresses"), "2^64");
        assert_eq!(subnet.scan_target, None);
        assert_eq!(value(&describe("::1/128").unwrap(), "Total addresses"), "1");
    }

    #[test]
    fn rejects_empty_and_garbage() {
        assert_eq!(describe("  "), Err(SubnetError::Empty));
        for input in [
            "hello",
            "10.0.0.1/33",
            "10.0.0.1 255.255.255.0 extra",
            "300.1.1.1/8",
        ] {
            assert_eq!(describe(input), Err(SubnetError::Invalid), "{input}");
        }
    }
}
