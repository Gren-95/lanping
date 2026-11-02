//! Developer CLI: `cargo run --example scan_cli -- 192.168.0.0/24 [ports] [banners]`

use lanping_core::ports::parse_ports;
use lanping_core::range::parse_targets;
use lanping_core::{ScanConfig, ScanEvent, start_scan};

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let target = args.next().expect("usage: scan_cli <target> [ports]");
    let targets = parse_targets(&target).expect("invalid target");
    let mut config = ScanConfig::new(targets);
    if let Some(ports) = args.next() {
        config.ports = parse_ports(&ports).expect("invalid ports");
        config.grab_banners = args.next().as_deref() == Some("banners");
    }

    let mut scan = start_scan(config);
    let mut hosts = std::collections::BTreeMap::new();
    while let Some(event) = scan.next_event().await {
        match event {
            ScanEvent::Host(host) => {
                hosts.insert(host.ip, *host);
            }
            ScanEvent::Progress { .. } => {}
            ScanEvent::Finished => break,
        }
    }
    for host in hosts.values() {
        println!(
            "{:<16} {:>9} {:<24} {:<18} {:<28} {:?}",
            host.ip,
            host.latency.map_or("-".to_owned(), |l| format!(
                "{:.1} ms",
                l.as_secs_f64() * 1000.0
            )),
            host.hostname.as_deref().unwrap_or("-"),
            host.mac.as_deref().unwrap_or("-"),
            host.vendor.as_deref().unwrap_or("-"),
            host.open_ports,
        );
        if host.discovery != Default::default() {
            println!("    {:?}", host.discovery);
        }
        for banner in &host.banners {
            println!("    {}: {}", banner.port, banner.text);
        }
    }
    println!("{} hosts", hosts.len());
}
