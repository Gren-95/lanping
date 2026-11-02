//! Filtering and ordering of the result list.

use std::cmp::Ordering;

use lanping_core::Host;
use lanping_core::device_type::guess_device_type;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Address,
    Latency,
    Hostname,
    Mac,
    Vendor,
    /// Number of open ports found.
    Ports,
    DeviceType,
}

impl SortKey {
    pub const ALL: [SortKey; 7] = [
        SortKey::Address,
        SortKey::Latency,
        SortKey::Hostname,
        SortKey::Mac,
        SortKey::Vendor,
        SortKey::Ports,
        SortKey::DeviceType,
    ];
}

impl std::fmt::Display for SortKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Address => "Address",
            Self::Latency => "Latency",
            Self::Hostname => "Hostname",
            Self::Mac => "MAC address",
            Self::Vendor => "Vendor",
            Self::Ports => "Open ports",
            Self::DeviceType => "Type",
        })
    }
}

pub fn compare_hosts(a: &Host, b: &Host, key: SortKey) -> Ordering {
    let by_address = a.ip.cmp(&b.ip);
    match key {
        SortKey::Address => by_address,
        SortKey::Latency => compare_optional(&a.latency, &b.latency).then(by_address),
        SortKey::Hostname => compare_optional_text(
            &a.display_name().map(str::to_owned),
            &b.display_name().map(str::to_owned),
        )
        .then(by_address),
        SortKey::Mac => compare_optional_text(&a.mac, &b.mac).then(by_address),
        SortKey::Vendor => compare_optional_text(&a.vendor, &b.vendor).then(by_address),
        SortKey::Ports => a.open_ports.len().cmp(&b.open_ports.len()).then(by_address),
        SortKey::DeviceType => guess_device_type(a)
            .cmp(&guess_device_type(b))
            .then(by_address),
    }
}

/// Orders present values first so missing data sinks to the bottom.
fn compare_optional<T: Ord>(a: &Option<T>, b: &Option<T>) -> Ordering {
    match (a, b) {
        (Some(a), Some(b)) => a.cmp(b),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn compare_optional_text(a: &Option<String>, b: &Option<String>) -> Ordering {
    compare_optional(
        &a.as_ref().map(|t| t.to_lowercase()),
        &b.as_ref().map(|t| t.to_lowercase()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn host(last_octet: u8) -> Host {
        Host::new(std::net::Ipv4Addr::new(10, 0, 0, last_octet))
    }

    #[test]
    fn missing_values_sort_last_in_ascending_order() {
        let mut fast = host(9);
        fast.latency = Some(Duration::from_millis(1));
        let silent = host(1);
        assert_eq!(
            compare_hosts(&fast, &silent, SortKey::Latency),
            Ordering::Less
        );
    }

    #[test]
    fn sorts_by_port_count_then_address() {
        let mut busy = host(9);
        busy.open_ports = vec![22, 80];
        let mut quiet = host(1);
        quiet.open_ports = vec![22];
        assert_eq!(compare_hosts(&quiet, &busy, SortKey::Ports), Ordering::Less);
        assert_eq!(
            compare_hosts(&host(1), &host(2), SortKey::Ports),
            Ordering::Less
        );
    }

    #[test]
    fn sorts_by_mac_with_missing_last() {
        let mut known = host(9);
        known.mac = Some("AA:00:00:00:00:01".into());
        assert_eq!(
            compare_hosts(&known, &host(1), SortKey::Mac),
            Ordering::Less
        );
    }

    #[test]
    fn addresses_sort_numerically() {
        assert_eq!(
            compare_hosts(&host(9), &host(10), SortKey::Address),
            Ordering::Less
        );
    }

    #[test]
    fn ties_fall_back_to_address() {
        assert_eq!(
            compare_hosts(&host(1), &host(2), SortKey::Vendor),
            Ordering::Less
        );
    }
}
