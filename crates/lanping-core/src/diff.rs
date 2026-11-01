//! Compares a finished scan with the previous one of the same network.

use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;

use crate::host::Host;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStatus {
    Unchanged,
    /// Not present in the previous scan.
    New,
    /// Same device, but its address, name or open ports differ.
    Changed,
}

#[derive(Debug, Default)]
pub struct ScanDiff {
    pub statuses: HashMap<Ipv4Addr, HostStatus>,
    /// Devices in the previous scan that did not answer this time.
    pub gone: Vec<Host>,
}

impl ScanDiff {
    pub fn status(&self, ip: Ipv4Addr) -> HostStatus {
        self.statuses.get(&ip).copied().unwrap_or(HostStatus::Unchanged)
    }

    pub fn count(&self, wanted: HostStatus) -> usize {
        self.statuses.values().filter(|status| **status == wanted).count()
    }
}

pub fn diff_scans(previous: &[Host], current: &[Host]) -> ScanDiff {
    let before: HashMap<String, &Host> = previous.iter().map(|host| (host.device_key(), host)).collect();
    let mut seen = HashSet::new();
    let mut statuses = HashMap::new();

    for host in current {
        let key = host.device_key();
        let status = match before.get(&key) {
            None => HostStatus::New,
            Some(old) if differs(old, host) => HostStatus::Changed,
            Some(_) => HostStatus::Unchanged,
        };
        seen.insert(key);
        statuses.insert(host.ip, status);
    }

    let mut gone: Vec<Host> = previous
        .iter()
        .filter(|host| !seen.contains(&host.device_key()))
        .cloned()
        .collect();
    gone.sort_by_key(|host| host.ip);
    ScanDiff { statuses, gone }
}

fn differs(old: &Host, new: &Host) -> bool {
    old.ip != new.ip || old.hostname != new.hostname || old.open_ports != new.open_ports
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(last_octet: u8, mac: Option<&str>) -> Host {
        let mut host = Host::new(Ipv4Addr::new(10, 0, 0, last_octet));
        host.mac = mac.map(str::to_owned);
        host
    }

    #[test]
    fn detects_new_gone_and_unchanged() {
        let previous = [host(1, Some("AA:00:00:00:00:01")), host(2, Some("AA:00:00:00:00:02"))];
        let current = [host(1, Some("AA:00:00:00:00:01")), host(3, Some("AA:00:00:00:00:03"))];
        let diff = diff_scans(&previous, &current);
        assert_eq!(diff.status(current[0].ip), HostStatus::Unchanged);
        assert_eq!(diff.status(current[1].ip), HostStatus::New);
        assert_eq!(diff.gone, vec![previous[1].clone()]);
        assert_eq!(diff.count(HostStatus::New), 1);
    }

    #[test]
    fn follows_a_device_that_moved_address() {
        let previous = [host(5, Some("AA:00:00:00:00:05"))];
        let current = [host(9, Some("AA:00:00:00:00:05"))];
        let diff = diff_scans(&previous, &current);
        assert_eq!(diff.status(current[0].ip), HostStatus::Changed);
        assert!(diff.gone.is_empty());
    }

    #[test]
    fn detects_name_and_port_changes() {
        let previous = [host(1, None)];
        let mut renamed = host(1, None);
        renamed.hostname = Some("printer".into());
        assert_eq!(diff_scans(&previous, &[renamed]).status("10.0.0.1".parse().unwrap()), HostStatus::Changed);
        let mut opened = host(1, None);
        opened.open_ports = vec![80];
        assert_eq!(diff_scans(&previous, &[opened]).status("10.0.0.1".parse().unwrap()), HostStatus::Changed);
    }

    #[test]
    fn hosts_without_mac_match_by_address() {
        let previous = [host(7, None)];
        let current = [host(7, None)];
        assert_eq!(diff_scans(&previous, &current).status(current[0].ip), HostStatus::Unchanged);
    }

    #[test]
    fn empty_previous_scan_marks_everything_new() {
        let current = [host(1, None)];
        assert_eq!(diff_scans(&[], &current).status(current[0].ip), HostStatus::New);
    }
}
