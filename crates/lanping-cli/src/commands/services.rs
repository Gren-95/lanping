//! `services`: what announces itself on the network.

use lanping_core::service_browse::{browse, kind_title};
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::{Table, print_json};

pub const INFO: Info = Info {
    name: "services",
    usage: "services [--json]",
    summary: "What announces itself over mDNS and SSDP. Listens for a few seconds.",
};

pub const SPEC: Spec = spec(&["json"], &[]);

pub async fn run(args: Parsed) -> Outcome {
    if !args.positionals.is_empty() {
        return Err(Failure::usage("services takes no arguments"));
    }
    let entries = browse().await;
    if wants_json(&args) {
        let list: Vec<_> = entries
            .iter()
            .map(|entry| {
                json!({
                    "kind": entry.kind,
                    "title": kind_title(&entry.kind),
                    "name": entry.display_name(),
                    "address": entry.address.map(|address| address.to_string()),
                    "port": entry.port,
                    "host": entry.host,
                    "details": entry.details,
                })
            })
            .collect();
        print_json(&json!(list));
    } else {
        let mut table = Table::new(&["KIND", "NAME", "ADDRESS", "PORT"]);
        for entry in &entries {
            table.row(vec![
                kind_title(&entry.kind),
                entry.display_name(),
                entry
                    .address
                    .map_or_else(|| "-".to_owned(), |address| address.to_string()),
                entry
                    .port
                    .map_or_else(|| "-".to_owned(), |port| port.to_string()),
            ]);
        }
        table.print();
    }
    Ok(Exit::Fine)
}
