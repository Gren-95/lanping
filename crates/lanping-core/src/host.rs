use std::net::{Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::discovery::Discovery;

/// What a service said when connected to, for example an SSH version string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceBanner {
    pub port: u16,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Host {
    pub ip: Ipv4Addr,
    pub latency: Option<Duration>,
    /// Time-to-live of the ping reply, when the OS reports it. Absent in older saved scans.
    #[serde(default)]
    pub ttl: Option<u8>,
    pub hostname: Option<String>,
    pub mac: Option<String>,
    pub vendor: Option<String>,
    pub open_ports: Vec<u16>,
    /// Absent in scans saved by older versions.
    #[serde(default)]
    pub banners: Vec<ServiceBanner>,
    /// IPv6 addresses the neighbor cache lists for this device. Absent in older saved scans.
    #[serde(default)]
    pub ipv6: Vec<Ipv6Addr>,
    /// What the device says about itself. Absent in older saved scans.
    #[serde(default)]
    pub discovery: Discovery,
}

impl Host {
    pub fn new(ip: Ipv4Addr) -> Self {
        Self {
            ip,
            latency: None,
            ttl: None,
            hostname: None,
            mac: None,
            vendor: None,
            open_ports: Vec::new(),
            banners: Vec::new(),
            ipv6: Vec::new(),
            discovery: Discovery::default(),
        }
    }

    /// Stable identity across scans: the MAC address when known, since
    /// DHCP can move a device to another IP, otherwise the IP itself.
    pub fn device_key(&self) -> String {
        self.mac.clone().unwrap_or_else(|| self.ip.to_string())
    }

    /// The best name known: the DNS hostname, else the one the device announces.
    pub fn display_name(&self) -> Option<&str> {
        self.hostname.as_deref().or(self.discovery.name.as_deref())
    }

    pub fn banner(&self, port: u16) -> Option<&str> {
        self.banners
            .iter()
            .find(|banner| banner.port == port)
            .map(|banner| banner.text.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_key_prefers_mac() {
        let mut host = Host::new("10.0.0.5".parse().unwrap());
        assert_eq!(host.device_key(), "10.0.0.5");
        host.mac = Some("AA:BB:CC:DD:EE:FF".into());
        assert_eq!(host.device_key(), "AA:BB:CC:DD:EE:FF");
    }

    #[test]
    fn loads_hosts_saved_without_banners() {
        let json = r#"{"ip":"10.0.0.5","latency":null,"hostname":null,"mac":null,"vendor":null,"open_ports":[22]}"#;
        let host: Host = serde_json::from_str(json).unwrap();
        assert!(host.banners.is_empty());
        assert!(host.ipv6.is_empty());
    }
}
