//! `email <domain>`: MX, SPF, DMARC and DKIM records of a domain.

use lanping_core::email_check::{RecordCheck, check};
use lanping_core::verdict::Verdict;
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, required, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::print_json;

pub const INFO: Info = Info {
    name: "email",
    usage: "email <domain> [--json]",
    summary: "MX, SPF, DMARC and DKIM records of a domain. Exit 3 when SPF or DMARC is missing or broken.",
};

pub const SPEC: Spec = spec(&["json"], &[]);

pub async fn run(args: Parsed) -> Outcome {
    let report = check(required(&args, "a domain")?)
        .await
        .map_err(Failure::failed)?;
    let problem =
        report.spf.verdict == Verdict::Missing || report.dmarc.verdict == Verdict::Missing;
    if wants_json(&args) {
        let servers: Vec<_> = report
            .mail_servers
            .iter()
            .map(|server| json!({"preference": server.preference, "host": server.host}))
            .collect();
        print_json(&json!({
            "domain": report.domain,
            "mail_servers": servers,
            "spf": record_json(&report.spf),
            "dmarc": record_json(&report.dmarc),
            "dkim_selectors": report.dkim_selectors,
        }));
    } else {
        for server in &report.mail_servers {
            println!("MX     {} {}", server.preference, server.host);
        }
        if report.mail_servers.is_empty() {
            println!("MX     none");
        }
        println!("SPF    {}", summary(&report.spf));
        println!("DMARC  {}", summary(&report.dmarc));
        let keys = if report.dkim_selectors.is_empty() {
            "none found at common names".to_owned()
        } else {
            report.dkim_selectors.join(", ")
        };
        println!("DKIM   {keys}");
    }
    Ok(if problem { Exit::Problem } else { Exit::Fine })
}

fn grade(check: &RecordCheck) -> &'static str {
    match (check.verdict, check.record.is_some()) {
        (Verdict::Good, _) => "good",
        (Verdict::Weak, _) => "weak",
        (Verdict::Missing, false) => "missing",
        (Verdict::Missing, true) => "problem",
    }
}

fn summary(check: &RecordCheck) -> String {
    format!("{}: {}", grade(check), check.meaning)
}

fn record_json(check: &RecordCheck) -> serde_json::Value {
    json!({"grade": grade(check), "record": check.record, "meaning": check.meaning})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check_of(verdict: Verdict, record: Option<&str>) -> RecordCheck {
        RecordCheck {
            verdict,
            record: record.map(str::to_owned),
            meaning: "m".to_owned(),
        }
    }

    #[test]
    fn a_broken_record_is_told_from_a_missing_one() {
        assert_eq!(grade(&check_of(Verdict::Missing, None)), "missing");
        assert_eq!(
            grade(&check_of(Verdict::Missing, Some("v=spf1 +all"))),
            "problem"
        );
        assert_eq!(summary(&check_of(Verdict::Good, Some("x"))), "good: m");
    }
}
