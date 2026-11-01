use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::probe::{TcpOutcome, tcp_probe};

/// Where the common port list comes from.
pub const PORT_LIST_SOURCE_URL: &str =
    "https://en.wikipedia.org/wiki/List_of_TCP_and_UDP_port_numbers";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortGroup {
    Web,
    RemoteAccess,
    FileSharing,
    Mail,
    Directory,
    Database,
    Printing,
    Messaging,
}

impl PortGroup {
    pub const ALL: [PortGroup; 8] = [
        PortGroup::Web,
        PortGroup::RemoteAccess,
        PortGroup::FileSharing,
        PortGroup::Mail,
        PortGroup::Directory,
        PortGroup::Database,
        PortGroup::Printing,
        PortGroup::Messaging,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Web => "Web",
            Self::RemoteAccess => "Remote access",
            Self::FileSharing => "File sharing",
            Self::Mail => "Mail",
            Self::Directory => "Directory and naming",
            Self::Database => "Databases",
            Self::Printing => "Printing",
            Self::Messaging => "Messaging and media",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Service {
    pub port: u16,
    pub name: &'static str,
    pub group: PortGroup,
}

const fn service(port: u16, name: &'static str, group: PortGroup) -> Service {
    Service { port, name, group }
}

/// TCP ports commonly found on home and office networks. Port numbers and
/// service names follow Wikipedia's "List of TCP and UDP port numbers"
/// (see [`PORT_LIST_SOURCE_URL`]).
pub const COMMON_SERVICES: [Service; 45] = [
    service(80, "HTTP", PortGroup::Web),
    service(443, "HTTPS", PortGroup::Web),
    service(8080, "HTTP alternate", PortGroup::Web),
    service(8443, "HTTPS alternate", PortGroup::Web),
    service(22, "SSH", PortGroup::RemoteAccess),
    service(23, "Telnet", PortGroup::RemoteAccess),
    service(3389, "RDP", PortGroup::RemoteAccess),
    service(5900, "VNC", PortGroup::RemoteAccess),
    service(1080, "SOCKS proxy", PortGroup::RemoteAccess),
    service(1723, "PPTP VPN", PortGroup::RemoteAccess),
    service(21, "FTP", PortGroup::FileSharing),
    service(139, "NetBIOS session", PortGroup::FileSharing),
    service(445, "SMB", PortGroup::FileSharing),
    service(548, "AFP", PortGroup::FileSharing),
    service(873, "rsync", PortGroup::FileSharing),
    service(2049, "NFS", PortGroup::FileSharing),
    service(25, "SMTP", PortGroup::Mail),
    service(110, "POP3", PortGroup::Mail),
    service(143, "IMAP", PortGroup::Mail),
    service(587, "SMTP submission", PortGroup::Mail),
    service(993, "IMAPS", PortGroup::Mail),
    service(995, "POP3S", PortGroup::Mail),
    service(53, "DNS", PortGroup::Directory),
    service(88, "Kerberos", PortGroup::Directory),
    service(111, "ONC RPC", PortGroup::Directory),
    service(135, "DCE / MS RPC", PortGroup::Directory),
    service(389, "LDAP", PortGroup::Directory),
    service(636, "LDAPS", PortGroup::Directory),
    service(1433, "Microsoft SQL Server", PortGroup::Database),
    service(3306, "MySQL", PortGroup::Database),
    service(5432, "PostgreSQL", PortGroup::Database),
    service(6379, "Redis", PortGroup::Database),
    service(27017, "MongoDB", PortGroup::Database),
    service(515, "LPD printing", PortGroup::Printing),
    service(631, "IPP printing", PortGroup::Printing),
    service(9100, "Raw printing (JetDirect)", PortGroup::Printing),
    service(119, "NNTP", PortGroup::Messaging),
    service(554, "RTSP streaming", PortGroup::Messaging),
    service(1883, "MQTT", PortGroup::Messaging),
    service(2375, "Docker API", PortGroup::Messaging),
    service(3689, "DAAP (iTunes)", PortGroup::Messaging),
    service(5060, "SIP", PortGroup::Messaging),
    service(5672, "AMQP", PortGroup::Messaging),
    service(8883, "MQTT over TLS", PortGroup::Messaging),
    service(32400, "Plex Media Server", PortGroup::Messaging),
];

pub fn common_ports() -> impl Iterator<Item = u16> {
    COMMON_SERVICES.iter().map(|service| service.port)
}

const MAX_PARALLEL_PROBES: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub struct PortListError(pub String);

impl std::fmt::Display for PortListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Cannot read port: {}", self.0)
    }
}

/// Parses `22, 80, 8000-8010` into a sorted, de-duplicated port list.
pub fn parse_ports(text: &str) -> Result<Vec<u16>, PortListError> {
    let mut ports = Vec::new();
    for entry in text.split(',').map(str::trim).filter(|e| !e.is_empty()) {
        let invalid = || PortListError(entry.to_owned());
        match entry.split_once('-') {
            Some((start, end)) => {
                let start: u16 = start.trim().parse().map_err(|_| invalid())?;
                let end: u16 = end.trim().parse().map_err(|_| invalid())?;
                if start == 0 || start > end {
                    return Err(invalid());
                }
                ports.extend(start..=end);
            }
            None => {
                let port: u16 = entry.parse().map_err(|_| invalid())?;
                if port == 0 {
                    return Err(invalid());
                }
                ports.push(port);
            }
        }
    }
    ports.sort_unstable();
    ports.dedup();
    Ok(ports)
}

pub fn format_ports(ports: &[u16]) -> String {
    ports
        .iter()
        .map(u16::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns the open ports of `ip`, sorted ascending.
pub async fn scan_ports(ip: Ipv4Addr, ports: &[u16], wait: Duration) -> Vec<u16> {
    let limit = std::sync::Arc::new(Semaphore::new(MAX_PARALLEL_PROBES));
    let mut attempts = JoinSet::new();
    for &port in ports {
        let limit = limit.clone();
        attempts.spawn(async move {
            let _permit = limit.acquire_owned().await.ok()?;
            let (outcome, _) = tcp_probe(ip, port, wait).await;
            (outcome == TcpOutcome::Open).then_some(port)
        });
    }

    let mut open = Vec::new();
    while let Some(joined) = attempts.join_next().await {
        if let Ok(Some(port)) = joined {
            open.push(port);
        }
    }
    open.sort_unstable();
    open
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn parses_ports_and_ranges() {
        assert_eq!(
            parse_ports("80, 22,8000-8002, 80").unwrap(),
            vec![22, 80, 8000, 8001, 8002]
        );
    }

    #[test]
    fn rejects_bad_ports() {
        assert!(parse_ports("0").is_err());
        assert!(parse_ports("70000").is_err());
        assert!(parse_ports("90-80").is_err());
        assert!(parse_ports("http").is_err());
    }

    #[test]
    fn common_services_are_unique_and_grouped() {
        let mut ports: Vec<u16> = common_ports().collect();
        ports.sort_unstable();
        let before = ports.len();
        ports.dedup();
        assert_eq!(ports.len(), before);
        for group in PortGroup::ALL {
            assert!(COMMON_SERVICES.iter().any(|service| service.group == group));
        }
    }

    #[test]
    fn empty_input_is_empty_list() {
        assert_eq!(parse_ports(" , ").unwrap(), Vec::<u16>::new());
    }

    #[tokio::test]
    async fn finds_only_listening_ports() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let open_port = listener.local_addr().unwrap().port();
        let closed_port = {
            let other = TcpListener::bind("127.0.0.1:0").await.unwrap();
            other.local_addr().unwrap().port()
        };
        let found = scan_ports(
            Ipv4Addr::LOCALHOST,
            &[open_port, closed_port],
            Duration::from_secs(1),
        )
        .await;
        assert_eq!(found, vec![open_port]);
    }
}
