//! `upnp`: the port forwards the router has open.

use lanping_core::upnp::read_port_mappings;
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::{Table, print_json};

pub const INFO: Info = Info {
    name: "upnp",
    usage: "upnp [--json]",
    summary: "The port forwards the router has open over UPnP. Read-only.",
};

pub const SPEC: Spec = spec(&["json"], &[]);

pub async fn run(args: Parsed) -> Outcome {
    if !args.positionals.is_empty() {
        return Err(Failure::usage("upnp takes no arguments"));
    }
    let report = read_port_mappings().await.map_err(Failure::failed)?;
    if wants_json(&args) {
        let mappings: Vec<_> = report
            .mappings
            .iter()
            .map(|m| {
                json!({
                    "protocol": m.protocol,
                    "external_port": m.external_port,
                    "internal_client": m.internal_client,
                    "internal_port": m.internal_port,
                    "description": m.description,
                    "enabled": m.enabled,
                    "lease_seconds": m.lease_seconds,
                    "remote_host": m.remote_host,
                })
            })
            .collect();
        print_json(&json!({
            "gateway": report.gateway.to_string(),
            "device": report.device.friendly_name,
            "truncated": report.truncated,
            "mappings": mappings,
        }));
    } else {
        let mut table = Table::new(&["PROTO", "OUTSIDE", "INSIDE", "LEASE", "DESCRIPTION"]);
        for m in &report.mappings {
            table.row(vec![
                m.protocol.clone(),
                m.external_port.to_string(),
                format!("{}:{}", m.internal_client, m.internal_port),
                if m.lease_seconds == 0 {
                    "none".to_owned()
                } else {
                    format!("{} s", m.lease_seconds)
                },
                m.description.clone(),
            ]);
        }
        table.print();
        eprintln!("{} forwards on {}", report.mappings.len(), report.gateway);
    }
    Ok(Exit::Fine)
}
