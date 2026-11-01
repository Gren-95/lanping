//! Unprivileged liveness probes: ICMP echo over datagram sockets, with TCP
//! connect probes for hosts that drop ping.

use std::io::ErrorKind;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use surge_ping::{Client, Config, ICMP, PingIdentifier, PingSequence};
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Ports tried when an address does not answer ping.
pub const LIVENESS_PORTS: [u16; 8] = [80, 443, 22, 445, 139, 135, 53, 8080];

const PING_PAYLOAD: [u8; 16] = [0; 16];

/// Builds the shared ICMP client. Returns `None` when the OS refuses an
/// unprivileged ICMP socket; scans then rely on TCP probes and the ARP cache.
pub fn create_icmp_client() -> Option<Client> {
    Client::new(&Config::builder().kind(ICMP::V4).build()).ok()
}

pub async fn ping(client: &Client, ip: Ipv4Addr, wait: Duration) -> Option<Duration> {
    let identifier = PingIdentifier(rand_identifier(ip));
    let mut pinger = client.pinger(IpAddr::V4(ip), identifier).await;
    pinger.timeout(wait);
    pinger
        .ping(PingSequence(0), &PING_PAYLOAD)
        .await
        .ok()
        .map(|(_packet, round_trip)| round_trip)
}

/// Identifier derived from the target so concurrent pings do not collide.
fn rand_identifier(ip: Ipv4Addr) -> u16 {
    let value = u32::from(ip);
    ((value >> 16) as u16) ^ (value as u16)
}

/// Result of one TCP connect attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TcpOutcome {
    Open,
    /// The host answered with a reset: it is alive but the port is closed.
    Refused,
    Silent,
}

pub async fn tcp_probe(ip: Ipv4Addr, port: u16, wait: Duration) -> (TcpOutcome, Duration) {
    let started = Instant::now();
    let address = SocketAddr::new(IpAddr::V4(ip), port);
    let outcome = match timeout(wait, TcpStream::connect(address)).await {
        Ok(Ok(_stream)) => TcpOutcome::Open,
        Ok(Err(error)) if error.kind() == ErrorKind::ConnectionRefused => TcpOutcome::Refused,
        _ => TcpOutcome::Silent,
    };
    (outcome, started.elapsed())
}

/// Tries the liveness ports in parallel. Returns the fastest sign of life.
pub async fn tcp_liveness(ip: Ipv4Addr, wait: Duration) -> Option<Duration> {
    let mut attempts = tokio::task::JoinSet::new();
    for port in LIVENESS_PORTS {
        attempts.spawn(tcp_probe(ip, port, wait));
    }
    while let Some(joined) = attempts.join_next().await {
        if let Ok((TcpOutcome::Open | TcpOutcome::Refused, elapsed)) = joined {
            attempts.abort_all();
            return Some(elapsed);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn detects_open_port() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (outcome, _) = tcp_probe(Ipv4Addr::LOCALHOST, port, Duration::from_secs(1)).await;
        assert_eq!(outcome, TcpOutcome::Open);
    }

    #[tokio::test]
    async fn detects_refused_port() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let (outcome, _) = tcp_probe(Ipv4Addr::LOCALHOST, port, Duration::from_secs(1)).await;
        assert_eq!(outcome, TcpOutcome::Refused);
    }
}
