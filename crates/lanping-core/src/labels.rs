//! User-chosen names for devices, keyed by [`crate::Host::device_key`].

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

const MAX_LABEL_CHARS: usize = 60;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeviceLabels(BTreeMap<String, String>);

impl DeviceLabels {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    /// Stores `label` for `key`. A blank label removes the entry.
    pub fn set(&mut self, key: &str, label: &str) {
        let trimmed: String = label.trim().chars().take(MAX_LABEL_CHARS).collect();
        if trimmed.is_empty() {
            self.0.remove(key);
        } else {
            self.0.insert(key.to_owned(), trimmed);
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let json = fs::read_to_string(path)?;
        serde_json::from_str(&json).map_err(io::Error::other)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_trims_and_removes() {
        let mut labels = DeviceLabels::default();
        labels.set("AA:BB", "  Living room TV ");
        assert_eq!(labels.get("AA:BB"), Some("Living room TV"));
        labels.set("AA:BB", "   ");
        assert_eq!(labels.get("AA:BB"), None);
        assert!(labels.is_empty());
    }

    #[test]
    fn limits_label_length() {
        let mut labels = DeviceLabels::default();
        labels.set("k", &"x".repeat(200));
        assert_eq!(labels.get("k").unwrap().chars().count(), MAX_LABEL_CHARS);
    }

    #[test]
    fn saves_and_loads() {
        let directory = std::env::temp_dir().join(format!("lanping-labels-{}", std::process::id()));
        let path = directory.join("labels.json");
        let mut labels = DeviceLabels::default();
        labels.set("AA:BB:CC:DD:EE:FF", "NAS");
        labels.save(&path).unwrap();
        assert_eq!(DeviceLabels::load(&path).unwrap(), labels);
        let _ = fs::remove_dir_all(&directory);
    }
}
