//! `ntp [server]`: how far this computer's clock is from a time server.

use lanping_core::ntp::{WELL_KNOWN_SERVERS, check};
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::{milliseconds, print_json};

pub const INFO: Info = Info {
    name: "ntp",
    usage: "ntp [server] [--warn SECONDS] [--json]",
    summary: "How far this computer's clock is off. Exit 3 when it is off by SECONDS (default 1) or more.",
};

pub const SPEC: Spec = spec(&["json"], &["warn"]);
const DEFAULT_WARN_SECONDS: f64 = 1.0;

pub async fn run(args: Parsed) -> Outcome {
    let server = args.first().unwrap_or(WELL_KNOWN_SERVERS[0]);
    let warn = match args.option("warn") {
        Some(text) => text
            .parse::<f64>()
            .ok()
            .filter(|seconds| *seconds >= 0.0)
            .ok_or_else(|| Failure::usage("--warn must be a number of seconds"))?,
        None => DEFAULT_WARN_SECONDS,
    };
    let report = check(server).await.map_err(Failure::failed)?;
    let off = report.clock_ahead_seconds.abs() >= warn;
    if wants_json(&args) {
        print_json(&json!({
            "server": report.server,
            "address": report.address.to_string(),
            "stratum": report.stratum,
            "clock_ahead_seconds": report.clock_ahead_seconds,
            "round_trip_seconds": report.round_trip_seconds,
            "off": off,
        }));
    } else {
        let direction = if report.clock_ahead_seconds >= 0.0 {
            "ahead of"
        } else {
            "behind"
        };
        println!(
            "This clock is {:.3} s {direction} {} ({})",
            report.clock_ahead_seconds.abs(),
            report.server,
            report.address
        );
        println!(
            "Round trip {}, stratum {}",
            milliseconds(report.round_trip_seconds),
            report.stratum
        );
    }
    Ok(if off { Exit::Problem } else { Exit::Fine })
}
