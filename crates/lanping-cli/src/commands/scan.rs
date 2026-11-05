//! `scan <target>`: the devices on a network.

use std::collections::BTreeMap;

use lanping_core::device_notes::DeviceNotes;
use lanping_core::export::to_json;
use lanping_core::ports::parse_ports;
use lanping_core::range::parse_targets;
use lanping_core::rtt_stats::millis;
use lanping_core::{Host, ScanConfig, ScanEvent, start_scan};

use super::{Exit, Failure, Info, Outcome, required, spec, wants_json};
use crate::args::{Parsed, Spec};
use crate::output::Table;

pub const INFO: Info = Info {
    name: "scan",
    usage: "scan <target> [--ports 22,80,443] [--banners] [--json]",
    summary: "The devices that answer on a range, such as 192.168.0.0/24.",
};

pub const SPEC: Spec = spec(&["json", "banners"], &["ports"]);

pub async fn run(args: Parsed) -> Outcome {
    let targets = parse_targets(required(&args, "a target such as 192.168.0.0/24")?)
        .map_err(Failure::usage)?;
    let mut config = ScanConfig::new(targets);
    if let Some(ports) = args.option("ports") {
        config.ports = parse_ports(ports).map_err(Failure::usage)?;
        config.grab_banners = args.switch("banners");
    } else if args.switch("banners") {
        return Err(Failure::usage("--banners needs --ports"));
    }
    let mut hosts = BTreeMap::new();
    let mut scan = start_scan(config);
    while let Some(event) = scan.next_event().await {
        match event {
            ScanEvent::Host(host) => {
                hosts.insert(host.ip, *host);
            }
            ScanEvent::Progress { .. } => {}
            ScanEvent::Finished => break,
        }
    }
    let hosts: Vec<Host> = hosts.into_values().collect();
    if wants_json(&args) {
        let json = to_json(&hosts, &DeviceNotes::default()).map_err(Failure::failed)?;
        println!("{json}");
    } else {
        table(&hosts).print();
        eprintln!("{} devices", hosts.len());
    }
    Ok(Exit::Fine)
}

fn table(hosts: &[Host]) -> Table {
    let mut table = Table::new(&["ADDRESS", "LATENCY", "NAME", "MAC", "VENDOR", "PORTS"]);
    for host in hosts {
        table.row(vec![
            host.ip.to_string(),
            host.latency.map_or_else(
                || "-".to_owned(),
                |latency| format!("{:.1} ms", millis(latency)),
            ),
            host.hostname.clone().unwrap_or_else(|| "-".to_owned()),
            host.mac.clone().unwrap_or_else(|| "-".to_owned()),
            host.vendor.clone().unwrap_or_else(|| "-".to_owned()),
            host.open_ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(","),
        ]);
    }
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn a_device_with_nothing_known_shows_dashes_and_no_ports() {
        let lines = table(&[Host::new(Ipv4Addr::new(10, 0, 0, 5))]).lines();
        assert_eq!(lines[1], "10.0.0.5  -        -     -    -");
    }

    #[test]
    fn open_ports_are_listed_with_commas() {
        let mut host = Host::new(Ipv4Addr::new(10, 0, 0, 5));
        host.open_ports = vec![22, 443];
        assert!(table(&[host]).lines()[1].ends_with("22,443"));
    }
}
