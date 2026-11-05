//! Saved scans, one JSON file per scan.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::IgnoredAny;
use serde::{Deserialize, Serialize};

use crate::files::{json_files, write_new_json};
use crate::host::Host;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedScan {
    pub target: String,
    pub finished_at_unix: u64,
    pub hosts: Vec<Host>,
}

impl SavedScan {
    pub fn new(target: String, hosts: Vec<Host>) -> Self {
        let finished_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        Self {
            target,
            finished_at_unix,
            hosts,
        }
    }
}

impl SavedScan {
    /// The device `device_key` as this scan saw it.
    pub fn host_for(&self, device_key: &str) -> Option<&Host> {
        self.hosts
            .iter()
            .find(|host| host.device_key() == device_key)
    }
}

/// What a list of saved scans needs to show: when, where and how many, without the hosts.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ScanSummary {
    pub target: String,
    pub finished_at_unix: u64,
    #[serde(rename = "hosts", deserialize_with = "count_items")]
    pub host_count: usize,
}

/// Counts a JSON array without building its items.
fn count_items<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<usize, D::Error> {
    Vec::<IgnoredAny>::deserialize(deserializer).map(|items| items.len())
}

/// One appearance of a device in a saved scan.
#[derive(Debug, Clone, PartialEq)]
pub struct Sighting {
    pub finished_at_unix: u64,
    pub target: String,
    pub host: Host,
}

/// Every scan in `scans` that contains the device `device_key`, in the order given.
pub fn sightings(scans: &[SavedScan], device_key: &str) -> Vec<Sighting> {
    scans
        .iter()
        .filter_map(|scan| {
            let host = scan.host_for(device_key)?;
            Some(Sighting {
                finished_at_unix: scan.finished_at_unix,
                target: scan.target.clone(),
                host: host.clone(),
            })
        })
        .collect()
}

/// Writes the scan into `directory` and returns the file path.
pub fn save_scan(directory: &Path, scan: &SavedScan) -> io::Result<PathBuf> {
    let json = serde_json::to_string(scan).map_err(io::Error::other)?;
    write_new_json(directory, &format!("scan-{}", scan.finished_at_unix), &json)
}

pub fn load_scan(path: &Path) -> io::Result<SavedScan> {
    let json = fs::read_to_string(path)?;
    serde_json::from_str(&json).map_err(io::Error::other)
}

/// Lists readable saved scans, newest first. Corrupt files are skipped.
pub fn list_scans(directory: &Path) -> Vec<(PathBuf, SavedScan)> {
    let mut scans: Vec<(PathBuf, SavedScan)> = json_files(directory)
        .into_iter()
        .filter_map(|path| load_scan(&path).ok().map(|scan| (path, scan)))
        .collect();
    scans.sort_by_key(|(_, scan)| std::cmp::Reverse(scan.finished_at_unix));
    scans
}

/// Lists readable saved scans as summaries, newest first. The hosts are skipped while reading,
/// so this is much cheaper than [`list_scans`] when only a list is needed.
pub fn list_scan_summaries(directory: &Path) -> Vec<(PathBuf, ScanSummary)> {
    let mut scans: Vec<(PathBuf, ScanSummary)> = json_files(directory)
        .into_iter()
        .filter_map(|path| {
            let json = fs::read_to_string(&path).ok()?;
            let summary = serde_json::from_str(&json).ok()?;
            Some((path, summary))
        })
        .collect();
    scans.sort_by_key(|(_, scan)| std::cmp::Reverse(scan.finished_at_unix));
    scans
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_scans_load_and_hold_only_made_up_macs() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/demo");
        let scans = list_scans(&directory);
        assert_eq!(scans.len(), 2);
        for (_, scan) in &scans {
            assert!(scan.hosts.iter().all(|host| {
                host.mac
                    .as_deref()
                    .is_some_and(|mac| mac.starts_with("02:00:5E:10"))
            }));
        }
    }

    #[test]
    fn summaries_match_the_full_scans_without_reading_hosts() {
        let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/demo");
        let full = list_scans(&directory);
        let summaries = list_scan_summaries(&directory);
        assert_eq!(full.len(), summaries.len());
        for ((full_path, scan), (summary_path, summary)) in full.iter().zip(&summaries) {
            assert_eq!(full_path, summary_path);
            assert_eq!(summary.target, scan.target);
            assert_eq!(summary.finished_at_unix, scan.finished_at_unix);
            assert_eq!(summary.host_count, scan.hosts.len());
        }
    }

    #[test]
    fn two_scans_finished_in_the_same_second_are_both_kept() {
        let directory = temp_directory("same-second");
        let scan = SavedScan {
            target: "10.0.0.0/24".into(),
            finished_at_unix: 7,
            hosts: Vec::new(),
        };
        save_scan(&directory, &scan).unwrap();
        save_scan(&directory, &scan).unwrap();
        assert_eq!(list_scan_summaries(&directory).len(), 2);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn finds_a_device_across_scans_by_mac_or_ip() {
        let mut printer = Host::new("10.0.0.5".parse().unwrap());
        printer.mac = Some("AA:BB:CC:DD:EE:FF".into());
        let mut moved = printer.clone();
        moved.ip = "10.0.0.9".parse().unwrap();
        let other = Host::new("10.0.0.7".parse().unwrap());
        let scans = [
            SavedScan {
                target: "10.0.0.0/24".into(),
                finished_at_unix: 300,
                hosts: vec![moved, other.clone()],
            },
            SavedScan {
                target: "10.0.0.0/24".into(),
                finished_at_unix: 200,
                hosts: vec![other],
            },
            SavedScan {
                target: "10.0.0.0/24".into(),
                finished_at_unix: 100,
                hosts: vec![printer],
            },
        ];
        let by_mac = sightings(&scans, "AA:BB:CC:DD:EE:FF");
        assert_eq!(
            by_mac
                .iter()
                .map(|s| s.finished_at_unix)
                .collect::<Vec<_>>(),
            [300, 100]
        );
        assert_eq!(sightings(&scans, "10.0.0.7").len(), 2);
        assert!(sightings(&scans, "10.0.0.99").is_empty());
    }

    fn temp_directory(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("lanping-test-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn saves_lists_and_loads() {
        let directory = temp_directory("history");
        let mut scan = SavedScan::new(
            "10.0.0.0/24".into(),
            vec![Host::new("10.0.0.1".parse().unwrap())],
        );
        scan.finished_at_unix = 100;
        let mut newer = scan.clone();
        newer.finished_at_unix = 200;

        let path = save_scan(&directory, &scan).unwrap();
        save_scan(&directory, &newer).unwrap();
        fs::write(directory.join("broken.json"), "not json").unwrap();

        assert_eq!(load_scan(&path).unwrap(), scan);
        let listed = list_scans(&directory);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].1.finished_at_unix, 200);
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn missing_directory_lists_nothing() {
        assert!(list_scans(&temp_directory("missing")).is_empty());
    }
}
