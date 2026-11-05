//! `web <address>`: how a website answers.

use lanping_core::verdict::Verdict;
use lanping_core::website_check::check;
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, required, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::{milliseconds, print_json};

pub const INFO: Info = Info {
    name: "web",
    usage: "web <address> [--json]",
    summary: "Redirects, timing, TLS version and security headers of a site. Exit 3 on an error status or a stalled redirect chain.",
};

pub const SPEC: Spec = spec(&["json"], &[]);
const FIRST_ERROR_STATUS: u16 = 400;

pub async fn run(args: Parsed) -> Outcome {
    let report = check(required(&args, "a web address")?)
        .await
        .map_err(Failure::failed)?;
    let status = report.hops.last().map_or(0, |hop| hop.status);
    let problem = status >= FIRST_ERROR_STATUS || report.stopped.is_some();
    let timings = &report.timings;
    if wants_json(&args) {
        let hops: Vec<_> = report
            .hops
            .iter()
            .map(|hop| json!({"url": hop.url, "status": hop.status, "location": hop.location, "ms": hop.total.as_secs_f64() * 1000.0}))
            .collect();
        let headers: Vec<_> = report
            .headers
            .iter()
            .map(|header| json!({"name": header.name, "value": header.value, "grade": grade(header.verdict), "advice": header.advice}))
            .collect();
        print_json(&json!({
            "status": status,
            "hops": hops,
            "tls_version": report.tls_version,
            "timings_ms": {
                "dns": timings.dns.as_secs_f64() * 1000.0,
                "connect": timings.connect.as_secs_f64() * 1000.0,
                "tls": timings.tls.map(|tls| tls.as_secs_f64() * 1000.0),
                "first_byte": timings.first_byte.as_secs_f64() * 1000.0,
                "total": timings.total.as_secs_f64() * 1000.0,
            },
            "headers": headers,
            "stopped": report.stopped,
        }));
    } else {
        for hop in &report.hops {
            let target = hop
                .location
                .as_deref()
                .map(|to| format!("  ->  {to}"))
                .unwrap_or_default();
            println!(
                "{}  {}{target}  ({})",
                hop.status,
                hop.url,
                milliseconds(hop.total.as_secs_f64())
            );
        }
        if let Some(reason) = &report.stopped {
            println!("Stopped: {reason}");
        }
        println!(
            "dns {}, connect {}, tls {}, first byte {}, total {}{}",
            milliseconds(timings.dns.as_secs_f64()),
            milliseconds(timings.connect.as_secs_f64()),
            timings
                .tls
                .map_or_else(|| "-".to_owned(), |tls| milliseconds(tls.as_secs_f64())),
            milliseconds(timings.first_byte.as_secs_f64()),
            milliseconds(timings.total.as_secs_f64()),
            report
                .tls_version
                .as_ref()
                .map(|version| format!("  ({version})"))
                .unwrap_or_default(),
        );
        for header in &report.headers {
            println!("{:<26} {}", header.name, grade(header.verdict));
        }
    }
    Ok(if problem { Exit::Problem } else { Exit::Fine })
}

fn grade(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Good => "good",
        Verdict::Weak => "weak",
        Verdict::Missing => "missing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_are_one_plain_word() {
        assert_eq!(grade(Verdict::Good), "good");
        assert_eq!(grade(Verdict::Weak), "weak");
        assert_eq!(grade(Verdict::Missing), "missing");
    }
}
