//! MAC address vendor lookup backed by the IEEE MA-L registry.

use std::collections::HashMap;
use std::sync::OnceLock;

const OUI_REGISTRY: &str = include_str!("../data/oui.tsv");

fn registry() -> &'static HashMap<&'static str, &'static str> {
    static REGISTRY: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        OUI_REGISTRY
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .collect()
    })
}

/// Looks up the vendor of a normalized MAC (`AA:BB:CC:DD:EE:FF`).
pub fn lookup_vendor(mac: &str) -> Option<&'static str> {
    let prefix: String = mac.split(':').take(3).collect();
    if prefix.len() != 6 {
        return None;
    }
    registry().get(prefix.as_str()).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_known_vendor() {
        assert_eq!(lookup_vendor("00:00:00:11:22:33"), Some("XEROX CORPORATION"));
    }

    #[test]
    fn returns_none_for_unknown_or_malformed() {
        assert_eq!(lookup_vendor("02:00:00:00:00:00"), None);
        assert_eq!(lookup_vendor("garbage"), None);
    }
}
