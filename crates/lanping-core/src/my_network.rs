//! The facts about this computer's connection that people look up all the time: the
//! address of each interface, the gateway, the DNS servers and the Wi-Fi name.
//!
//! Everything here is read from this computer. Nothing is sent over the network.

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use crate::command::run;
use crate::dns::parse_resolv_conf;
use crate::gateway::default_gateway;

const RESOLV_CONF: &str = "/etc/resolv.conf";
/// Where systemd-resolved keeps the servers behind its local stub address.
const RESOLVED_CONF: &str = "/run/systemd/resolve/resolv.conf";
const TOOL_WAIT: Duration = Duration::from_secs(3);

/// One address of an interface, with the length of its network prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InterfaceAddress {
    pub ip: IpAddr,
    pub prefix: u8,
}

impl InterfaceAddress {
    /// `192.168.0.5/24`.
    pub fn with_prefix(&self) -> String {
        format!("{}/{}", self.ip, self.prefix)
    }

    /// An IPv6 address that only works on the link it is on.
    pub fn is_link_local(&self) -> bool {
        match self.ip {
            IpAddr::V4(ip) => ip.is_link_local(),
            IpAddr::V6(ip) => ip.segments()[0] & 0xffc0 == 0xfe80,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceInfo {
    pub name: String,
    pub addresses: Vec<InterfaceAddress>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFacts {
    pub interfaces: Vec<InterfaceInfo>,
    pub gateway: Option<Ipv4Addr>,
    pub dns_servers: Vec<IpAddr>,
    /// The network this computer is joined to over Wi-Fi, when it is.
    pub wifi_name: Option<String>,
}

/// Reads the facts from this computer.
pub async fn local_facts() -> LocalFacts {
    let gateway = tokio::task::spawn_blocking(default_gateway)
        .await
        .ok()
        .flatten();
    LocalFacts {
        interfaces: interfaces(),
        gateway,
        dns_servers: dns_servers().await,
        wifi_name: wifi_name().await,
    }
}

/// The interfaces with an address, loopback left out, in name order. Addresses are grouped
/// per interface, IPv4 before IPv6.
fn interfaces() -> Vec<InterfaceInfo> {
    let Ok(found) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    group_by_interface(
        found
            .into_iter()
            .filter(|interface| !interface.is_loopback())
            .map(|interface| {
                let prefix = match &interface.addr {
                    if_addrs::IfAddr::V4(v4) => u32::from(v4.netmask).leading_ones() as u8,
                    if_addrs::IfAddr::V6(v6) => u128::from(v6.netmask).leading_ones() as u8,
                };
                (
                    interface.name.clone(),
                    InterfaceAddress {
                        ip: interface.ip(),
                        prefix,
                    },
                )
            }),
    )
}

fn group_by_interface(
    addresses: impl IntoIterator<Item = (String, InterfaceAddress)>,
) -> Vec<InterfaceInfo> {
    let mut interfaces: Vec<InterfaceInfo> = Vec::new();
    for (name, address) in addresses {
        match interfaces.iter_mut().find(|known| known.name == name) {
            Some(known) => known.addresses.push(address),
            None => interfaces.push(InterfaceInfo {
                name,
                addresses: vec![address],
            }),
        }
    }
    for interface in &mut interfaces {
        interface
            .addresses
            .sort_by_key(|address| address.ip.is_ipv6());
    }
    interfaces.sort_by(|a, b| a.name.cmp(&b.name));
    interfaces
}

async fn dns_servers() -> Vec<IpAddr> {
    if cfg!(windows) {
        return match run("ipconfig", &["/all"], TOOL_WAIT).await {
            Ok((_, output)) => parse_ipconfig_dns(&output),
            Err(_) => Vec::new(),
        };
    }
    let configured = fs::read_to_string(RESOLV_CONF)
        .map(|text| without_repeats(parse_resolv_conf(&text)))
        .unwrap_or_default();
    // A stub on this computer forwards to the servers that matter.
    if configured.iter().all(|server| server.is_loopback())
        && let Ok(text) = fs::read_to_string(RESOLVED_CONF)
    {
        let behind_stub = without_repeats(parse_resolv_conf(&text));
        if !behind_stub.is_empty() {
            return behind_stub;
        }
    }
    configured
}

/// `servers` without repeats, in order.
fn without_repeats(servers: Vec<IpAddr>) -> Vec<IpAddr> {
    let mut unique: Vec<IpAddr> = Vec::with_capacity(servers.len());
    for server in servers {
        if !unique.contains(&server) {
            unique.push(server);
        }
    }
    unique
}

/// The addresses after `DNS Servers` in `ipconfig /all`, which go on over the lines below.
fn parse_ipconfig_dns(text: &str) -> Vec<IpAddr> {
    let mut servers: Vec<IpAddr> = Vec::new();
    let mut in_list = false;
    for line in text.lines() {
        let value = match line.split_once(':') {
            Some((label, rest)) if label.contains("DNS Servers") => {
                in_list = true;
                rest.trim()
            }
            _ if in_list && line.starts_with(char::is_whitespace) => line.trim(),
            _ => {
                in_list = false;
                continue;
            }
        };
        match value.split('%').next().and_then(|word| word.parse().ok()) {
            Some(address) if !servers.contains(&address) => servers.push(address),
            Some(_) => {}
            None => in_list = false,
        }
    }
    servers
}

async fn wifi_name() -> Option<String> {
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        ("netsh", &["wlan", "show", "interfaces"])
    } else if cfg!(target_os = "macos") {
        return None;
    } else {
        ("nmcli", &["-t", "-f", "ACTIVE,SSID", "dev", "wifi"])
    };
    let (_, output) = run(program, args, TOOL_WAIT).await.ok()?;
    if cfg!(windows) {
        parse_netsh_ssid(&output)
    } else {
        parse_nmcli_active(&output)
    }
}

/// The name on the `yes:` line of `nmcli -t -f ACTIVE,SSID dev wifi`. A colon in the name
/// is written `\:`.
fn parse_nmcli_active(text: &str) -> Option<String> {
    let name = text
        .lines()
        .find_map(|line| line.strip_prefix("yes:"))?
        .replace("\\:", ":")
        .replace("\\\\", "\\");
    let name = name.trim();
    (!name.is_empty()).then(|| name.to_owned())
}

/// The `SSID` line of `netsh wlan show interfaces`, not the `BSSID` one.
fn parse_netsh_ssid(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (label, value) = line.split_once(':')?;
        let value = value.trim();
        (label.trim() == "SSID" && !value.is_empty()).then(|| value.to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(ip: &str, prefix: u8) -> InterfaceAddress {
        InterfaceAddress {
            ip: ip.parse().unwrap(),
            prefix,
        }
    }

    #[test]
    fn addresses_are_grouped_per_interface_with_ipv4_first() {
        let grouped = group_by_interface([
            ("wlan0".to_owned(), address("fe80::1", 64)),
            ("eth0".to_owned(), address("10.0.0.5", 24)),
            ("wlan0".to_owned(), address("192.168.0.9", 24)),
        ]);
        let shape: Vec<(&str, Vec<String>)> = grouped
            .iter()
            .map(|i| {
                (
                    i.name.as_str(),
                    i.addresses
                        .iter()
                        .map(InterfaceAddress::with_prefix)
                        .collect(),
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                ("eth0", vec!["10.0.0.5/24".to_owned()]),
                (
                    "wlan0",
                    vec!["192.168.0.9/24".to_owned(), "fe80::1/64".to_owned()]
                ),
            ]
        );
    }

    #[test]
    fn link_local_addresses_are_recognised() {
        assert!(address("fe80::1", 64).is_link_local());
        assert!(address("169.254.3.4", 16).is_link_local());
        assert!(!address("2001:db8::1", 64).is_link_local());
        assert!(!address("192.168.0.9", 24).is_link_local());
    }

    #[test]
    fn a_server_listed_twice_is_shown_once() {
        let text = "nameserver 192.168.0.1\nnameserver 1.1.1.1\nnameserver 192.168.0.1\n";
        assert_eq!(
            without_repeats(parse_resolv_conf(text)),
            [
                "192.168.0.1".parse::<IpAddr>().unwrap(),
                "1.1.1.1".parse().unwrap(),
            ]
        );
    }

    #[test]
    fn windows_lists_servers_on_the_lines_after_the_first() {
        let text = "Ethernet adapter Ethernet:\n\n   DNS Servers . . . . . . . . . . . : 192.168.0.1\n\
                    \x20                                      8.8.8.8\n   NetBIOS over Tcpip. . . . . . . . : Enabled\n";
        assert_eq!(
            parse_ipconfig_dns(text),
            [
                "192.168.0.1".parse::<IpAddr>().unwrap(),
                "8.8.8.8".parse().unwrap()
            ]
        );
    }

    #[test]
    fn the_joined_wifi_network_is_the_active_one() {
        assert_eq!(
            parse_nmcli_active("no:Neighbour\nyes:Home\\: 5G\nno:Cafe\n").as_deref(),
            Some("Home: 5G")
        );
        assert_eq!(parse_nmcli_active("no:Neighbour\n"), None);
        assert_eq!(parse_nmcli_active("yes:\n"), None);
    }

    #[test]
    fn netsh_ssid_is_not_mistaken_for_the_bssid() {
        let text = "    Name                   : Wi-Fi\n    SSID                   : Home\n    \
                    BSSID                  : aa:bb:cc:dd:ee:ff\n";
        assert_eq!(parse_netsh_ssid(text).as_deref(), Some("Home"));
        assert_eq!(
            parse_netsh_ssid("    There is 1 interface on the system"),
            None
        );
    }

    #[tokio::test]
    async fn the_facts_can_be_read_on_this_computer() {
        let facts = local_facts().await;
        assert!(facts.interfaces.iter().all(|i| !i.addresses.is_empty()));
    }
}
