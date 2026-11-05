//! What changed about a device over time, worked out from the saved scans.

use std::collections::HashSet;
use std::fmt;
use std::net::Ipv4Addr;

use crate::history::SavedScan;
use crate::host::Host;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    FirstSeen,
    WentOffline,
    CameBack,
    AddressChanged(Ipv4Addr),
    NameChanged(Option<String>),
    PortOpened(u16),
    PortClosed(u16),
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FirstSeen => f.write_str("First seen"),
            Self::WentOffline => f.write_str("Went offline"),
            Self::CameBack => f.write_str("Came back online"),
            Self::AddressChanged(ip) => write!(f, "Address changed to {ip}"),
            Self::NameChanged(Some(name)) => write!(f, "Name changed to {name}"),
            Self::NameChanged(None) => f.write_str("Name no longer known"),
            Self::PortOpened(port) => write!(f, "Port {port} opened"),
            Self::PortClosed(port) => write!(f, "Port {port} closed"),
        }
    }
}

/// One change and the finish time of the scan that first showed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEntry {
    pub at_unix: u64,
    pub change: Change,
}

/// Changes between two sightings of the same device.
fn changes_between(before: &Host, after: &Host) -> Vec<Change> {
    let mut changes = Vec::new();
    if before.ip != after.ip {
        changes.push(Change::AddressChanged(after.ip));
    }
    if before.hostname != after.hostname {
        changes.push(Change::NameChanged(after.hostname.clone()));
    }
    // A scan with no open ports at all may have been run with port scanning off,
    // so it says nothing about individual ports.
    if !before.open_ports.is_empty() && !after.open_ports.is_empty() {
        let was_open: HashSet<u16> = before.open_ports.iter().copied().collect();
        let is_open: HashSet<u16> = after.open_ports.iter().copied().collect();
        let mut opened: Vec<u16> = is_open.difference(&was_open).copied().collect();
        let mut closed: Vec<u16> = was_open.difference(&is_open).copied().collect();
        opened.sort_unstable();
        closed.sort_unstable();
        changes.extend(opened.into_iter().map(Change::PortOpened));
        changes.extend(closed.into_iter().map(Change::PortClosed));
    }
    changes
}

/// The device's history, newest first. `scans` may be in any order. Only scans of
/// networks the device has been seen on count, so a scan of another network does
/// not make it look offline.
pub fn device_timeline(scans: &[SavedScan], device_key: &str) -> Vec<TimelineEntry> {
    let networks: HashSet<&str> = scans
        .iter()
        .filter(|scan| scan.host_for(device_key).is_some())
        .map(|scan| scan.target.as_str())
        .collect();
    let mut relevant: Vec<&SavedScan> = scans
        .iter()
        .filter(|scan| networks.contains(scan.target.as_str()))
        .collect();
    relevant.sort_by_key(|scan| scan.finished_at_unix);

    let mut entries = Vec::new();
    let mut last_seen: Option<&Host> = None;
    let mut online = false;
    for scan in relevant {
        let at_unix = scan.finished_at_unix;
        let mut record = |change| entries.push(TimelineEntry { at_unix, change });
        match (scan.host_for(device_key), last_seen) {
            (Some(host), None) => {
                record(Change::FirstSeen);
                last_seen = Some(host);
                online = true;
            }
            (Some(host), Some(before)) => {
                if !online {
                    record(Change::CameBack);
                }
                changes_between(before, host)
                    .into_iter()
                    .for_each(&mut record);
                last_seen = Some(host);
                online = true;
            }
            (None, Some(_)) if online => {
                record(Change::WentOffline);
                online = false;
            }
            (None, _) => {}
        }
    }
    entries.reverse();
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "02:00:5E:10:00:01";

    fn device(last_octet: u8, ports: &[u16]) -> Host {
        let mut host = Host::new(Ipv4Addr::new(10, 0, 0, last_octet));
        host.mac = Some(KEY.to_owned());
        host.open_ports = ports.to_vec();
        host
    }

    fn scan(at: u64, hosts: Vec<Host>) -> SavedScan {
        SavedScan {
            target: "10.0.0.0/24".into(),
            finished_at_unix: at,
            hosts,
        }
    }

    fn changes(scans: &[SavedScan]) -> Vec<(u64, Change)> {
        device_timeline(scans, KEY)
            .into_iter()
            .map(|entry| (entry.at_unix, entry.change))
            .collect()
    }

    #[test]
    fn reports_ports_and_address_newest_first() {
        let scans = [
            scan(200, vec![device(9, &[22, 80])]),
            scan(100, vec![device(5, &[22, 443])]),
        ];
        assert_eq!(
            changes(&scans),
            [
                (200, Change::PortClosed(443)),
                (200, Change::PortOpened(80)),
                (200, Change::AddressChanged(Ipv4Addr::new(10, 0, 0, 9))),
                (100, Change::FirstSeen),
            ]
        );
    }

    #[test]
    fn reports_going_offline_and_coming_back_once_each() {
        let scans = [
            scan(100, vec![device(5, &[])]),
            scan(200, vec![]),
            scan(300, vec![]),
            scan(400, vec![device(5, &[])]),
        ];
        assert_eq!(
            changes(&scans),
            [
                (400, Change::CameBack),
                (200, Change::WentOffline),
                (100, Change::FirstSeen),
            ]
        );
    }

    #[test]
    fn a_scan_without_ports_says_nothing_about_ports() {
        let scans = [
            scan(100, vec![device(5, &[22])]),
            scan(200, vec![device(5, &[])]),
            scan(300, vec![device(5, &[22])]),
        ];
        assert_eq!(changes(&scans), [(100, Change::FirstSeen)]);
    }

    #[test]
    fn scans_of_other_networks_do_not_count() {
        let mut elsewhere = scan(200, vec![]);
        elsewhere.target = "172.16.0.0/24".into();
        let scans = [scan(100, vec![device(5, &[])]), elsewhere];
        assert_eq!(changes(&scans), [(100, Change::FirstSeen)]);
    }

    #[test]
    fn earlier_scans_before_the_first_sighting_are_ignored() {
        let scans = [scan(100, vec![]), scan(200, vec![device(5, &[])])];
        assert_eq!(changes(&scans), [(200, Change::FirstSeen)]);
        assert!(device_timeline(&scans, "nobody").is_empty());
    }
}
