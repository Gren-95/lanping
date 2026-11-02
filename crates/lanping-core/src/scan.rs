//! Scan orchestration. A scan runs in four stages: sweep, neighbor-cache
//! merge, then name / vendor / port enrichment. Each stage streams updates.

use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use surge_ping::Client;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::timeout;

use crate::banner::grab_banner;
use crate::discovery::query_host;
use crate::host::{Host, ServiceBanner};
use crate::neighbors::{read_ipv6_neighbors, read_neighbor_table};
use crate::ports::scan_ports;
use crate::probe::{create_icmp_client, ping, tcp_liveness};
use crate::ssdp;
use crate::vendor::lookup_vendor;

const EVENT_BUFFER: usize = 512;
const PROGRESS_STEP: usize = 16;
const MAX_PARALLEL_LOOKUPS: usize = 64;
const HOSTNAME_TIMEOUT: Duration = Duration::from_secs(2);
const DISCOVERY_TIMEOUT: Duration = Duration::from_millis(800);
const SSDP_LISTEN_TIME: Duration = Duration::from_millis(1500);

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub targets: Vec<Ipv4Addr>,
    /// TCP ports to check on every live host. Empty disables the port scan.
    pub ports: Vec<u16>,
    /// Ask devices about themselves over mDNS, NetBIOS and SSDP.
    pub discover: bool,
    /// Read the greeting of every open port found by the port scan.
    pub grab_banners: bool,
    pub probe_timeout: Duration,
    pub max_parallel_probes: usize,
}

impl ScanConfig {
    pub fn new(targets: Vec<Ipv4Addr>) -> Self {
        Self {
            targets,
            ports: Vec::new(),
            discover: true,
            grab_banners: false,
            probe_timeout: Duration::from_millis(800),
            max_parallel_probes: 256,
        }
    }
}

/// The part of a scan a progress report describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanStage {
    /// Probing every target address to find live hosts.
    Sweep,
    /// Looking up names and checking ports on the hosts that were found.
    Details,
}

#[derive(Debug, Clone)]
pub enum ScanEvent {
    Progress {
        stage: ScanStage,
        scanned: usize,
        total: usize,
    },
    /// A live host was found, or new details arrived for a known one.
    Host(Box<Host>),
    Finished,
}

/// A running scan. Dropping the handle cancels it.
pub struct ScanHandle {
    events: mpsc::Receiver<ScanEvent>,
    task: JoinHandle<()>,
}

impl ScanHandle {
    pub async fn next_event(&mut self) -> Option<ScanEvent> {
        self.events.recv().await
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Starts a scan on the current tokio runtime.
pub fn start_scan(config: ScanConfig) -> ScanHandle {
    let (sender, events) = mpsc::channel(EVENT_BUFFER);
    let task = tokio::spawn(run_scan(config, sender));
    ScanHandle { events, task }
}

async fn run_scan(config: ScanConfig, events: mpsc::Sender<ScanEvent>) {
    let mut known = sweep(&config, &events).await;
    merge_neighbors(&config, &mut known, &events).await;
    enrich(&config, known, &events).await;
    let _ = events.send(ScanEvent::Finished).await;
}

async fn sweep(config: &ScanConfig, events: &mpsc::Sender<ScanEvent>) -> HashMap<Ipv4Addr, Host> {
    let icmp: Option<Arc<Client>> = create_icmp_client().map(Arc::new);
    let limit = Arc::new(Semaphore::new(config.max_parallel_probes.max(1)));
    let total = config.targets.len();

    let mut probes = JoinSet::new();
    for &ip in &config.targets {
        let (icmp, limit, wait) = (icmp.clone(), limit.clone(), config.probe_timeout);
        probes.spawn(async move {
            let _permit = limit.acquire_owned().await.ok()?;
            probe_host(icmp.as_deref(), ip, wait).await
        });
    }

    let mut known = HashMap::new();
    let mut scanned = 0;
    while let Some(joined) = probes.join_next().await {
        scanned += 1;
        if let Ok(Some(host)) = joined {
            known.insert(host.ip, host.clone());
            if events.send(ScanEvent::Host(Box::new(host))).await.is_err() {
                return known;
            }
        }
        if scanned % PROGRESS_STEP == 0 || scanned == total {
            let progress = ScanEvent::Progress {
                stage: ScanStage::Sweep,
                scanned,
                total,
            };
            if events.send(progress).await.is_err() {
                return known;
            }
        }
    }
    known
}

async fn probe_host(icmp: Option<&Client>, ip: Ipv4Addr, wait: Duration) -> Option<Host> {
    let answer = match icmp {
        Some(client) => ping(client, ip, wait).await,
        None => None,
    };
    let (latency, ttl) = match answer {
        Some(answer) => answer,
        None => (tcp_liveness(ip, wait).await?, None),
    };
    let mut host = Host::new(ip);
    host.latency = Some(latency);
    host.ttl = ttl;
    Some(host)
}

/// Adds MAC addresses from the OS neighbor cache. Hosts that ignored every
/// probe but resolved over ARP are added as live hosts without latency.
async fn merge_neighbors(
    config: &ScanConfig,
    known: &mut HashMap<Ipv4Addr, Host>,
    events: &mpsc::Sender<ScanEvent>,
) {
    let table = read_neighbor_table().await;
    let ipv6 = read_ipv6_neighbors().await;
    for &ip in &config.targets {
        let Some(mac) = table.get(&ip) else { continue };
        let host = known.entry(ip).or_insert_with(|| Host::new(ip));
        host.vendor = lookup_vendor(mac).map(str::to_owned);
        host.ipv6 = ipv6.get(mac).cloned().unwrap_or_default();
        host.mac = Some(mac.clone());
        if events
            .send(ScanEvent::Host(Box::new(host.clone())))
            .await
            .is_err()
        {
            return;
        }
    }
}

async fn enrich(
    config: &ScanConfig,
    known: HashMap<Ipv4Addr, Host>,
    events: &mpsc::Sender<ScanEvent>,
) {
    let total = known.len();
    let report = |done| ScanEvent::Progress {
        stage: ScanStage::Details,
        scanned: done,
        total,
    };
    if total > 0 && events.send(report(0)).await.is_err() {
        return;
    }
    let discover = config.discover;
    let upnp_targets: Vec<Ipv4Addr> = known.keys().copied().collect();
    let survey = tokio::spawn(async move {
        if discover {
            ssdp::survey(&upnp_targets, SSDP_LISTEN_TIME).await
        } else {
            HashMap::new()
        }
    });
    let limit = Arc::new(Semaphore::new(MAX_PARALLEL_LOOKUPS));
    let mut lookups = JoinSet::new();
    for (_ip, mut host) in known {
        let (limit, ports, wait) = (limit.clone(), config.ports.clone(), config.probe_timeout);
        let grab = config.grab_banners;
        lookups.spawn(async move {
            let _permit = limit.acquire_owned().await.ok()?;
            host.hostname = resolve_hostname(host.ip).await;
            if discover {
                host.discovery = query_host(host.ip, DISCOVERY_TIMEOUT).await;
            }
            if !ports.is_empty() {
                host.open_ports = scan_ports(host.ip, &ports, wait).await;
                if grab {
                    host.banners = collect_banners(host.ip, &host.open_ports, wait).await;
                }
            }
            Some(host)
        });
    }

    let mut done = 0;
    let mut finished = HashMap::new();
    while let Some(joined) = lookups.join_next().await {
        done += 1;
        if let Ok(Some(host)) = joined {
            finished.insert(host.ip, host.clone());
            if events.send(ScanEvent::Host(Box::new(host))).await.is_err() {
                return;
            }
        }
        if events.send(report(done)).await.is_err() {
            return;
        }
    }

    // UPnP answers arrive on their own schedule, so they update hosts already sent.
    for (ip, description) in survey.await.unwrap_or_default() {
        if let Some(mut host) = finished.remove(&ip) {
            host.discovery.add_description(description);
            if events.send(ScanEvent::Host(Box::new(host))).await.is_err() {
                return;
            }
        }
    }
}

async fn collect_banners(ip: Ipv4Addr, ports: &[u16], wait: Duration) -> Vec<ServiceBanner> {
    let mut attempts = JoinSet::new();
    for &port in ports {
        attempts.spawn(async move {
            grab_banner(ip, port, wait)
                .await
                .map(|text| ServiceBanner { port, text })
        });
    }
    let mut banners = Vec::new();
    while let Some(joined) = attempts.join_next().await {
        if let Ok(Some(banner)) = joined {
            banners.push(banner);
        }
    }
    banners.sort_by_key(|banner| banner.port);
    banners
}

async fn resolve_hostname(ip: Ipv4Addr) -> Option<String> {
    let lookup = tokio::task::spawn_blocking(move || dns_lookup::lookup_addr(&ip.into()));
    let name = timeout(HOSTNAME_TIMEOUT, lookup).await.ok()?.ok()?.ok()?;
    let is_numeric_fallback = name.parse::<std::net::IpAddr>().is_ok();
    (!name.is_empty() && !is_numeric_fallback).then_some(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn finds_loopback_and_reports_progress() {
        let mut config = ScanConfig::new(vec![Ipv4Addr::LOCALHOST]);
        config.probe_timeout = Duration::from_millis(500);
        config.discover = false;
        let mut scan = start_scan(config);

        let mut found_loopback = false;
        let mut saw_full_progress = false;
        let mut saw_full_details = false;
        let mut finished = false;
        while let Some(event) = scan.next_event().await {
            match event {
                ScanEvent::Host(host) => found_loopback |= host.ip == Ipv4Addr::LOCALHOST,
                ScanEvent::Progress {
                    stage,
                    scanned,
                    total,
                } => {
                    let full = scanned == total && total == 1;
                    match stage {
                        ScanStage::Sweep => saw_full_progress |= full,
                        ScanStage::Details => saw_full_details |= full,
                    }
                }
                ScanEvent::Finished => finished = true,
            }
        }
        assert!(found_loopback, "loopback should answer ICMP or refuse TCP");
        assert!(saw_full_progress);
        assert!(saw_full_details);
        assert!(finished);
    }

    #[tokio::test]
    async fn dropping_the_handle_stops_the_scan() {
        let targets: Vec<Ipv4Addr> = (1..=200).map(|n| Ipv4Addr::new(192, 0, 2, n)).collect();
        let scan = start_scan(ScanConfig::new(targets));
        drop(scan);
    }
}
