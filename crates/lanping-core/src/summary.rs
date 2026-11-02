//! Totals across the listed devices, for the summary dialog.

use std::collections::{BTreeMap, HashMap};

use crate::device_type::{DeviceType, guess_device_type};
use crate::history::SavedScan;
use crate::host::Host;

const SECONDS_PER_WEEK: u64 = 7 * 24 * 60 * 60;
const UNKNOWN_VENDOR: &str = "Unknown vendor";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub device_count: usize,
    pub by_type: Vec<(DeviceType, usize)>,
    pub by_vendor: Vec<(String, usize)>,
    pub by_port: Vec<(u16, usize)>,
    /// Listed devices whose first sighting in any saved scan is under a week old.
    pub new_this_week: usize,
}

/// Largest count first; ties keep the order of the key.
fn ranked<K: Ord + Clone>(counts: &BTreeMap<K, usize>) -> Vec<(K, usize)> {
    let mut rows: Vec<(K, usize)> = counts
        .iter()
        .map(|(key, count)| (key.clone(), *count))
        .collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    rows
}

pub fn summarize(hosts: &[Host], scans: &[SavedScan], now_unix: u64) -> Summary {
    let mut types = BTreeMap::new();
    let mut vendors = BTreeMap::new();
    let mut ports = BTreeMap::new();
    for host in hosts {
        *types.entry(guess_device_type(host)).or_insert(0) += 1;
        let vendor = host
            .vendor
            .clone()
            .unwrap_or_else(|| UNKNOWN_VENDOR.to_owned());
        *vendors.entry(vendor).or_insert(0) += 1;
        for port in &host.open_ports {
            *ports.entry(*port).or_insert(0) += 1;
        }
    }

    let mut first_seen: HashMap<String, u64> = HashMap::new();
    for scan in scans {
        for host in &scan.hosts {
            first_seen
                .entry(host.device_key())
                .and_modify(|at| *at = (*at).min(scan.finished_at_unix))
                .or_insert(scan.finished_at_unix);
        }
    }
    let week_ago = now_unix.saturating_sub(SECONDS_PER_WEEK);
    let new_this_week = hosts
        .iter()
        .filter(|host| {
            first_seen
                .get(&host.device_key())
                .is_none_or(|at| *at >= week_ago)
        })
        .count();

    Summary {
        device_count: hosts.len(),
        by_type: ranked(&types),
        by_vendor: ranked(&vendors),
        by_port: ranked(&ports),
        new_this_week,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 10 * SECONDS_PER_WEEK;

    fn host(last_octet: u8, vendor: Option<&str>, ports: &[u16]) -> Host {
        let mut found = Host::new(std::net::Ipv4Addr::new(10, 0, 0, last_octet));
        found.vendor = vendor.map(str::to_owned);
        found.open_ports = ports.to_vec();
        found
    }

    fn scan(at: u64, hosts: Vec<Host>) -> SavedScan {
        SavedScan {
            target: "10.0.0.0/24".into(),
            finished_at_unix: at,
            hosts,
        }
    }

    #[test]
    fn counts_and_ranks_vendors_ports_and_types() {
        let hosts = [
            host(1, Some("Acme"), &[80, 22]),
            host(2, Some("Acme"), &[80, 9100]),
            host(3, None, &[]),
        ];
        let summary = summarize(&hosts, &[], NOW);
        assert_eq!(summary.device_count, 3);
        assert_eq!(
            summary.by_vendor,
            [("Acme".to_owned(), 2), (UNKNOWN_VENDOR.to_owned(), 1)]
        );
        assert_eq!(summary.by_port[0], (80, 2));
        assert_eq!(summary.by_port.len(), 3);
        assert_eq!(summary.by_type.iter().map(|(_, n)| n).sum::<usize>(), 3);
    }

    #[test]
    fn new_this_week_uses_the_first_sighting_in_any_scan() {
        let old = host(1, None, &[]);
        let recent = host(2, None, &[]);
        let never_saved = host(3, None, &[]);
        let scans = [
            scan(NOW - 2 * SECONDS_PER_WEEK, vec![old.clone()]),
            scan(NOW - 60, vec![old.clone(), recent.clone()]),
        ];
        let summary = summarize(&[old, recent, never_saved], &scans, NOW);
        assert_eq!(summary.new_this_week, 2);
    }

    #[test]
    fn an_empty_list_is_all_zeros() {
        assert_eq!(summarize(&[], &[], NOW), Summary::default());
    }
}
