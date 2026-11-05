//! `tls <host[:port]>`: the certificate a server presents.

use lanping_core::timestamp::format_unix_utc;
use lanping_core::tls::{Trust, check};
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, required, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::print_json;

pub const INFO: Info = Info {
    name: "tls",
    usage: "tls <host[:port]> [--name SERVER_NAME] [--days N] [--json]",
    summary: "The certificate chain of a server. Exit 3 when it is not trusted or expires within N days (default 14).",
};

pub const SPEC: Spec = spec(&["json"], &["name", "days"]);
const DEFAULT_WARN_DAYS: i64 = 14;

pub async fn run(args: Parsed) -> Outcome {
    let target = required(&args, "a host")?;
    let warn_days = match args.option("days") {
        Some(text) => text
            .parse::<i64>()
            .ok()
            .filter(|days| *days >= 0)
            .ok_or_else(|| Failure::usage("--days must be a whole number"))?,
        None => DEFAULT_WARN_DAYS,
    };
    let report = check(target, args.option("name"))
        .await
        .map_err(Failure::failed)?;
    let days_left = report
        .chain
        .first()
        .map(|leaf| leaf.days_left(report.checked_at));
    let trusted = matches!(report.trust, Trust::Trusted);
    let expiring = days_left.is_some_and(|days| days < warn_days);
    if wants_json(&args) {
        let chain: Vec<_> = report
            .chain
            .iter()
            .map(|cert| {
                json!({
                    "subject": cert.subject,
                    "issuer": cert.issuer,
                    "not_before": cert.not_before,
                    "not_after": cert.not_after,
                    "days_left": cert.days_left(report.checked_at),
                    "serial": cert.serial,
                    "names": cert.names,
                    "self_signed": cert.self_signed,
                })
            })
            .collect();
        print_json(&json!({
            "host": report.host,
            "port": report.port,
            "address": report.address.map(|address| address.ip().to_string()),
            "protocol": report.protocol,
            "cipher": report.cipher,
            "trust": trust_text(&report.trust),
            "trusted": trusted,
            "days_left": days_left,
            "chain": chain,
        }));
    } else {
        println!(
            "{}:{}  {}",
            report.host,
            report.port,
            trust_text(&report.trust)
        );
        if let Some(protocol) = &report.protocol {
            println!("{protocol} {}", report.cipher.as_deref().unwrap_or(""));
        }
        for (index, cert) in report.chain.iter().enumerate() {
            println!("[{index}] {}", cert.subject);
            println!("    issued by {}", cert.issuer);
            println!(
                "    valid {} to {} ({} days left)",
                format_unix_utc(u64::try_from(cert.not_before).unwrap_or(0)),
                format_unix_utc(u64::try_from(cert.not_after).unwrap_or(0)),
                cert.days_left(report.checked_at)
            );
        }
    }
    Ok(if trusted && !expiring {
        Exit::Fine
    } else {
        Exit::Problem
    })
}

fn trust_text(trust: &Trust) -> String {
    match trust {
        Trust::Trusted => "trusted".to_owned(),
        Trust::SelfSigned => "self-signed".to_owned(),
        Trust::Problem(reason) => format!("not trusted: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_reads_in_plain_words() {
        assert_eq!(trust_text(&Trust::Trusted), "trusted");
        assert_eq!(trust_text(&Trust::SelfSigned), "self-signed");
        assert_eq!(
            trust_text(&Trust::Problem("expired".to_owned())),
            "not trusted: expired"
        );
    }
}
