//! `dns <name>`: DNS records, asked of this computer's resolver or one given.

use std::net::IpAddr;

use lanping_core::dns::{PUBLIC_RESOLVERS, RecordType, lookup, system_resolvers};
use serde_json::json;

use super::{Exit, Failure, Info, Outcome, required, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::{Table, milliseconds, print_json};

pub const INFO: Info = Info {
    name: "dns",
    usage: "dns <name> [--type A|AAAA|MX|TXT|NS|PTR|CNAME] [--server ADDRESS] [--json]",
    summary: "DNS records of a name. An address is looked up in reverse with --type PTR.",
};

pub const SPEC: Spec = spec(&["json"], &["type", "server"]);

pub async fn run(args: Parsed) -> Outcome {
    let name = required(&args, "a name")?;
    let record_type = match args.option("type") {
        Some(text) => parse_type(text)?,
        None => RecordType::A,
    };
    let server = match args.option("server") {
        Some(text) => text
            .parse::<IpAddr>()
            .map_err(|_| Failure::usage("--server must be an IP address"))?,
        None => system_resolvers()
            .first()
            .copied()
            .unwrap_or(PUBLIC_RESOLVERS[0].1),
    };
    let answer = lookup(server, name, record_type)
        .await
        .map_err(Failure::failed)?;
    if wants_json(&args) {
        let records: Vec<_> = answer
            .records
            .iter()
            .map(|record| json!({"type": record.type_name(), "ttl": record.ttl, "data": record.data}))
            .collect();
        print_json(&json!({
            "name": name,
            "server": answer.server.to_string(),
            "elapsed_ms": answer.elapsed.as_secs_f64() * 1000.0,
            "records": records,
        }));
    } else {
        let mut table = Table::new(&["TYPE", "TTL", "DATA"]);
        for record in &answer.records {
            table.row(vec![
                record.type_name(),
                record.ttl.to_string(),
                record.data.clone(),
            ]);
        }
        table.print();
        eprintln!(
            "{} answered in {}",
            answer.server,
            milliseconds(answer.elapsed.as_secs_f64())
        );
    }
    Ok(Exit::Fine)
}

fn parse_type(text: &str) -> Result<RecordType, Failure> {
    RecordType::ALL
        .into_iter()
        .find(|kind| kind.to_string().eq_ignore_ascii_case(text))
        .ok_or_else(|| Failure::usage(format!("unknown record type {text}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_types_are_read_in_any_case() {
        assert_eq!(parse_type("mx").unwrap(), RecordType::Mx);
        assert_eq!(parse_type("AAAA").unwrap(), RecordType::Aaaa);
        assert!(parse_type("SRV").unwrap_err().usage);
    }
}
