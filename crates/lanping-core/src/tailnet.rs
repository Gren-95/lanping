//! Tailscale peers, read from the local `tailscale` command line tool.

use std::net::Ipv4Addr;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

const STATUS_TIMEOUT: Duration = Duration::from_secs(2);

/// IPv4 addresses of this device's online Tailscale peers. Empty when
/// Tailscale is missing, stopped, or too slow to answer.
pub fn online_peers() -> Vec<Ipv4Addr> {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let output = Command::new("tailscale").args(["status", "--json"]).output();
        let _ = sender.send(output);
    });
    match receiver.recv_timeout(STATUS_TIMEOUT) {
        Ok(Ok(output)) if output.status.success() => {
            parse_online_peers(&String::from_utf8_lossy(&output.stdout))
        }
        _ => Vec::new(),
    }
}

/// Reads `tailscale status --json`. Offline peers are skipped because they
/// cannot answer a scan.
pub fn parse_online_peers(json: &str) -> Vec<Ipv4Addr> {
    let Ok(status) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    let Some(peers) = status.get("Peer").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut addresses: Vec<Ipv4Addr> = peers
        .values()
        .filter(|peer| peer.get("Online").and_then(Value::as_bool) == Some(true))
        .filter_map(|peer| peer.get("TailscaleIPs")?.as_array())
        .flatten()
        .filter_map(|address| address.as_str()?.parse().ok())
        .collect();
    addresses.sort_unstable();
    addresses
}

#[cfg(test)]
mod tests {
    use super::*;

    const STATUS: &str = r#"{
        "BackendState": "Running",
        "Self": {"TailscaleIPs": ["100.84.190.99", "fd7a::1"]},
        "Peer": {
            "a": {"Online": true,  "TailscaleIPs": ["100.93.72.103", "fd7a::2"]},
            "b": {"Online": false, "TailscaleIPs": ["100.100.178.62", "fd7a::3"]},
            "c": {"Online": true,  "TailscaleIPs": ["100.92.89.63"]}
        }
    }"#;

    #[test]
    fn keeps_only_online_ipv4_peers() {
        let peers = parse_online_peers(STATUS);
        assert_eq!(
            peers,
            vec!["100.92.89.63".parse::<Ipv4Addr>().unwrap(), "100.93.72.103".parse().unwrap()]
        );
    }

    #[test]
    fn tolerates_missing_peers_and_bad_json() {
        assert!(parse_online_peers(r#"{"BackendState": "Stopped"}"#).is_empty());
        assert!(parse_online_peers("not json").is_empty());
    }
}
