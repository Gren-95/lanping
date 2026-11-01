//! Saved scans, one JSON file per scan.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

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

/// Writes the scan into `directory` and returns the file path.
pub fn save_scan(directory: &Path, scan: &SavedScan) -> io::Result<PathBuf> {
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("scan-{}.json", scan.finished_at_unix));
    let json = serde_json::to_string(scan).map_err(io::Error::other)?;
    fs::write(&path, json)?;
    Ok(path)
}

pub fn load_scan(path: &Path) -> io::Result<SavedScan> {
    let json = fs::read_to_string(path)?;
    serde_json::from_str(&json).map_err(io::Error::other)
}

/// Lists readable saved scans, newest first. Corrupt files are skipped.
pub fn list_scans(directory: &Path) -> Vec<(PathBuf, SavedScan)> {
    let Ok(entries) = fs::read_dir(directory) else {
        return Vec::new();
    };
    let mut scans: Vec<(PathBuf, SavedScan)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .filter_map(|path| load_scan(&path).ok().map(|scan| (path, scan)))
        .collect();
    scans.sort_by_key(|(_, scan)| std::cmp::Reverse(scan.finished_at_unix));
    scans
}

#[cfg(test)]
mod tests {
    use super::*;

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
