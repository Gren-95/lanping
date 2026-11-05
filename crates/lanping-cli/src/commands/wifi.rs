//! `wifi`: the Wi-Fi networks in range.

use lanping_core::wifi::scan_access_points;
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::{Table, print_json};

pub const INFO: Info = Info {
    name: "wifi",
    usage: "wifi [--json]",
    summary: "The Wi-Fi networks in range, strongest first.",
};

pub const SPEC: Spec = spec(&["json"], &[]);

pub async fn run(args: Parsed) -> Outcome {
    if !args.positionals.is_empty() {
        return Err(Failure::usage("wifi takes no arguments"));
    }
    let access_points = scan_access_points().await.map_err(Failure::failed)?;
    if wants_json(&args) {
        let list: Vec<_> = access_points
            .iter()
            .map(|ap| {
                json!({
                    "ssid": ap.ssid,
                    "bssid": ap.bssid,
                    "channel": ap.channel,
                    "frequency_mhz": ap.frequency_mhz,
                    "band": ap.band.to_string(),
                    "signal_percent": ap.signal_percent,
                    "security": ap.security,
                    "rate_mbps": ap.rate_mbps,
                })
            })
            .collect();
        print_json(&json!(list));
    } else {
        let mut table = Table::new(&["SSID", "BSSID", "CHANNEL", "BAND", "SIGNAL", "SECURITY"]);
        for ap in &access_points {
            table.row(vec![
                if ap.ssid.is_empty() {
                    "(hidden)".to_owned()
                } else {
                    ap.ssid.clone()
                },
                ap.bssid.clone(),
                ap.channel.to_string(),
                ap.band.to_string(),
                format!("{}%", ap.signal_percent),
                if ap.security.is_empty() {
                    "open".to_owned()
                } else {
                    ap.security.clone()
                },
            ]);
        }
        table.print();
    }
    Ok(Exit::Fine)
}
