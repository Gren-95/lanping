//! Latency logger: probes one host again and again and keeps the answers.

use std::collections::VecDeque;
use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::ops::Deref;
use std::time::Duration;

use surge_ping::Client;

use crate::probe::{create_icmp_client, ping, tcp_liveness};
use crate::rtt_stats::RttStats;
use crate::trace::validate_target;

/// How long one probe waits for an answer.
const PROBE_WAIT: Duration = Duration::from_secs(2);
/// Probes drawn in the history graph.
pub const GRAPH_LENGTH: usize = 120;
/// Probes kept for the saved log: a day at one probe a second.
const MAX_LOGGED_PROBES: usize = 86_400;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LatencyError {
    InvalidTarget,
    NotFound,
}

impl fmt::Display for LatencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTarget => f.write_str("Enter a host name or an IPv4 address"),
            Self::NotFound => f.write_str("No IPv4 address found for that name"),
        }
    }
}

/// Sends probes to one IPv4 address. ICMP ping when the system allows it without
/// root, otherwise TCP connects to common ports.
#[derive(Clone)]
pub struct Prober {
    address: Ipv4Addr,
    client: Option<Client>,
}

impl fmt::Debug for Prober {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Prober")
            .field("address", &self.address)
            .field("method", &self.method())
            .finish()
    }
}

impl Prober {
    /// Resolves `target` and prepares the probe socket.
    pub async fn connect(target: &str) -> Result<Self, LatencyError> {
        let target = validate_target(target)
            .map_err(|_| LatencyError::InvalidTarget)?
            .to_owned();
        let address = resolve_ipv4(target).await?;
        Ok(Self {
            address,
            client: create_icmp_client(),
        })
    }

    pub fn address(&self) -> Ipv4Addr {
        self.address
    }

    pub fn method(&self) -> &'static str {
        if self.client.is_some() {
            "ICMP ping"
        } else {
            "TCP connect"
        }
    }

    /// One probe. The round trip in milliseconds, or `None` when nothing answered.
    pub async fn sample(&self) -> Option<f64> {
        let round_trip = match &self.client {
            Some(client) => ping(client, self.address, PROBE_WAIT)
                .await
                .map(|(round_trip, _)| round_trip),
            None => tcp_liveness(self.address, PROBE_WAIT).await,
        };
        round_trip.map(|round_trip| round_trip.as_secs_f64() * 1000.0)
    }
}

async fn resolve_ipv4(target: String) -> Result<Ipv4Addr, LatencyError> {
    if let Ok(address) = target.parse::<Ipv4Addr>() {
        return Ok(address);
    }
    tokio::task::spawn_blocking(move || dns_lookup::lookup_host(&target))
        .await
        .ok()
        .and_then(Result::ok)
        .and_then(|addresses| {
            addresses.into_iter().find_map(|address| match address {
                IpAddr::V4(address) => Some(address),
                IpAddr::V6(_) => None,
            })
        })
        .ok_or(LatencyError::NotFound)
}

/// One logged probe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LoggedProbe {
    pub unix_seconds: u64,
    pub rtt_ms: Option<f64>,
}

/// Statistics, the graph history and the full log of one run.
/// The statistics are reachable directly: `series.sent`, `series.average_ms()`.
#[derive(Debug, Clone, PartialEq)]
pub struct LatencySeries {
    stats: RttStats,
    log: VecDeque<LoggedProbe>,
}

impl Default for LatencySeries {
    fn default() -> Self {
        Self {
            stats: RttStats::new(GRAPH_LENGTH),
            log: VecDeque::new(),
        }
    }
}

impl Deref for LatencySeries {
    type Target = RttStats;

    fn deref(&self) -> &RttStats {
        &self.stats
    }
}

impl LatencySeries {
    pub fn record(&mut self, unix_seconds: u64, rtt_ms: Option<f64>) {
        self.stats.record(rtt_ms);
        self.log.push_back(LoggedProbe {
            unix_seconds,
            rtt_ms,
        });
        if self.log.len() > MAX_LOGGED_PROBES {
            self.log.pop_front();
        }
    }

    pub fn logged(&self) -> usize {
        self.log.len()
    }

    /// The log as CSV: `unix_time,rtt_ms`, with an empty time for a probe with no answer.
    pub fn log_as_csv(&self) -> String {
        let lines = self.log.iter().map(|probe| {
            let rtt = probe
                .rtt_ms
                .map_or_else(String::new, |rtt| format!("{rtt:.3}"));
            format!("{},{rtt}", probe.unix_seconds)
        });
        std::iter::once("unix_time,rtt_ms".to_owned())
            .chain(lines)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn probing_the_loopback_address_gets_an_answer() {
        let prober = Prober::connect("127.0.0.1").await.unwrap();
        assert_eq!(prober.address(), Ipv4Addr::LOCALHOST);
        // Without ICMP permission this falls back to TCP, which may find nothing listening.
        if prober.method() == "ICMP ping" {
            assert!(prober.sample().await.is_some());
        }
    }

    #[tokio::test]
    async fn refuses_options_and_unknown_names() {
        assert_eq!(
            Prober::connect("-x").await.unwrap_err(),
            LatencyError::InvalidTarget
        );
        assert_eq!(
            Prober::connect("nonexistent.invalid").await.unwrap_err(),
            LatencyError::NotFound
        );
    }

    #[test]
    fn the_series_counts_and_logs_every_probe() {
        let mut series = LatencySeries::default();
        series.record(100, Some(2.5));
        series.record(101, None);
        assert_eq!(series.sent, 2);
        assert_eq!(series.loss_percent(), 50.0);
        assert_eq!(series.logged(), 2);
        assert_eq!(series.log_as_csv(), "unix_time,rtt_ms\n100,2.500\n101,");
    }

    #[test]
    fn the_log_is_bounded() {
        let mut series = LatencySeries::default();
        for second in 0..MAX_LOGGED_PROBES as u64 + 3 {
            series.record(second, Some(1.0));
        }
        assert_eq!(series.logged(), MAX_LOGGED_PROBES);
        assert!(series.log_as_csv().contains("\n3,1.000"));
    }
}
