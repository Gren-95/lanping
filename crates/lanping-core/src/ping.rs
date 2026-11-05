//! Ping for the speed tests: how long a TCP connection to the test server takes to open.
//! It works for any host and port, needs no privileges, and says how far away a server is.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::net::{TcpStream, lookup_host};
use tokio::time::{sleep, timeout};

use crate::rtt_stats::{RttStats, millis};
use crate::socket_budget::socket_permit;

/// Connections opened for one ping measurement.
pub const PROBES: usize = 6;
const PROBE_WAIT: Duration = Duration::from_secs(2);
const PROBE_PAUSE: Duration = Duration::from_millis(120);
const LOOKUP_WAIT: Duration = Duration::from_secs(5);

/// What a ping measurement found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PingSummary {
    pub probes: u32,
    pub answered: u32,
    pub average_ms: Option<f64>,
    pub best_ms: Option<f64>,
    pub worst_ms: Option<f64>,
    /// Mean change between one answer and the next.
    pub jitter_ms: Option<f64>,
}

impl PingSummary {
    pub fn from_stats(stats: &RttStats) -> Self {
        Self {
            probes: stats.sent,
            answered: stats.received,
            average_ms: stats.average_ms(),
            best_ms: stats.best_ms,
            worst_ms: stats.worst_ms,
            jitter_ms: stats.jitter_ms(),
        }
    }

    /// `12.3 ms`, or `no answer` when every probe was lost.
    pub fn describe(&self) -> String {
        self.average_ms
            .map_or_else(|| "no answer".to_owned(), |ms| format!("{ms:.1} ms"))
    }
}

/// Times [`PROBES`] connections to `host:port`. The name is looked up once first, so the
/// lookup does not count. Probes that get no answer count as lost, which is a result, not an error.
pub async fn tcp_ping(host: &str, port: u16) -> Result<PingSummary, String> {
    let address = resolve(host, port).await?;
    let mut stats = RttStats::new(PROBES);
    for probe in 0..PROBES {
        if probe > 0 {
            sleep(PROBE_PAUSE).await;
        }
        stats.record(connect_time(address).await);
    }
    Ok(PingSummary::from_stats(&stats))
}

async fn resolve(host: &str, port: u16) -> Result<SocketAddr, String> {
    timeout(LOOKUP_WAIT, lookup_host((host, port)))
        .await
        .map_err(|_| format!("Looking up {host} took too long"))?
        .map_err(|_| format!("Could not find the host name \"{host}\""))?
        .next()
        .ok_or_else(|| format!("Could not find the host name \"{host}\""))
}

/// Milliseconds a connection took to open. A refused connection still counts: the host
/// answered. `None` when nothing answered in time.
async fn connect_time(address: SocketAddr) -> Option<f64> {
    let _permit = socket_permit().await;
    let started = Instant::now();
    let outcome = timeout(PROBE_WAIT, TcpStream::connect(address))
        .await
        .ok()?;
    let answered = match outcome {
        Ok(_) => true,
        Err(error) => error.kind() == std::io::ErrorKind::ConnectionRefused,
    };
    answered.then(|| millis(started.elapsed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn pings_a_listening_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { while listener.accept().await.is_ok() {} });
        let summary = tcp_ping("127.0.0.1", port).await.unwrap();
        assert_eq!(summary.probes as usize, PROBES);
        assert_eq!(summary.answered as usize, PROBES);
        assert!(summary.average_ms.unwrap() < 1000.0);
        assert!(summary.best_ms.unwrap() <= summary.worst_ms.unwrap());
        assert!(summary.jitter_ms.is_some());
    }

    #[tokio::test]
    async fn a_closed_port_still_shows_the_host_is_there() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let summary = tcp_ping("127.0.0.1", port).await.unwrap();
        assert_eq!(summary.answered as usize, PROBES);
    }

    #[tokio::test]
    async fn an_unknown_name_is_an_error() {
        let error = tcp_ping("nonexistent.invalid", 443).await.unwrap_err();
        assert!(error.contains("nonexistent.invalid"), "{error}");
    }

    #[test]
    fn describes_the_average_or_the_loss() {
        let mut stats = RttStats::new(3);
        stats.record(Some(10.0));
        stats.record(Some(20.0));
        assert_eq!(PingSummary::from_stats(&stats).describe(), "15.0 ms");
        let lost = PingSummary::from_stats(&RttStats::new(3));
        assert_eq!(lost.describe(), "no answer");
    }
}
