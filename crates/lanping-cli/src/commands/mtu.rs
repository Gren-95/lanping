//! `mtu <host>`: the largest packet that reaches a host without being split.

use lanping_core::path_mtu::{Method, check, usual_name};
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, required, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::print_json;

pub const INFO: Info = Info {
    name: "mtu",
    usage: "mtu <host> [--json]",
    summary: "The largest packet that reaches a host without being split.",
};

pub const SPEC: Spec = spec(&["json"], &[]);

pub async fn run(args: Parsed) -> Outcome {
    let report = check(required(&args, "a host")?)
        .await
        .map_err(Failure::failed)?;
    let method = match report.method {
        Method::Tracepath => "tracepath",
        Method::PingSearch => "ping",
    };
    if wants_json(&args) {
        print_json(&json!({
            "host": report.target,
            "mtu": report.mtu,
            "at_least": report.at_least,
            "method": method,
            "usual": usual_name(report.mtu),
            "narrowed_at_hop": report.narrowed_at.as_ref().map(|hop| hop.number),
            "narrowed_at_address": report.narrowed_at.as_ref().and_then(|hop| hop.address.clone()),
        }));
    } else {
        let more = if report.at_least { " or more" } else { "" };
        let usual = usual_name(report.mtu)
            .map(|name| format!(" ({name})"))
            .unwrap_or_default();
        println!(
            "{}: {}{more}{usual}, measured with {method}",
            report.target, report.mtu
        );
        if let Some(hop) = &report.narrowed_at {
            println!(
                "Narrowed at hop {} {}",
                hop.number,
                hop.address.as_deref().unwrap_or("")
            );
        }
    }
    Ok(Exit::Fine)
}
