//! The "Open with" menu for one host.

use std::net::Ipv4Addr;

use lanping_core::Host;
use lanping_core::diagnostics::Diagnostic;

use crate::opener::share_target;

const WEB_PORTS: [(u16, bool); 4] = [(80, false), (443, true), (8080, false), (8443, true)];
/// Ports that speak TLS from the first byte and are offered a certificate check when open.
const TLS_PORTS: [u16; 2] = [443, 8443];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostAction {
    OpenWeb {
        https: bool,
        port: u16,
    },
    BrowseFiles,
    CopySshCommand,
    CopyAddress,
    CopyMac,
    CopyHostname,
    WakeOnLan,
    Ping,
    Traceroute,
    /// Read the TLS certificate the host presents on this port.
    CheckCertificate {
        port: u16,
    },
}

/// A heading in the "Open with" menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionGroup {
    Open,
    Network,
    Copy,
}

impl ActionGroup {
    /// Menu order.
    pub const ALL: [ActionGroup; 3] = [ActionGroup::Open, ActionGroup::Network, ActionGroup::Copy];

    pub fn title(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Network => "Network",
            Self::Copy => "Copy",
        }
    }
}

/// What the app has to do to carry out an action.
#[derive(Debug, PartialEq, Eq)]
pub enum Effect {
    Open(String),
    Copy(String),
    /// Send a Wake-on-LAN packet for this MAC address.
    Wake(String),
    /// Run ping or traceroute against this address and show the output.
    Diagnose(Diagnostic, Ipv4Addr),
    /// Check the TLS certificate at this `address:port`.
    CheckCertificate(String),
}

impl HostAction {
    pub fn label(&self) -> String {
        match self {
            Self::OpenWeb { https, port } => {
                let scheme = if *https { "https" } else { "http" };
                format!("Open in browser ({scheme}:{port})")
            }
            Self::BrowseFiles => "Browse shared files (SMB)".to_owned(),
            Self::CopySshCommand => "Copy SSH command".to_owned(),
            Self::CopyAddress => "Copy IP address".to_owned(),
            Self::CopyMac => "Copy MAC address".to_owned(),
            Self::CopyHostname => "Copy hostname".to_owned(),
            Self::WakeOnLan => "Wake on LAN".to_owned(),
            Self::Ping => "Ping".to_owned(),
            Self::Traceroute => "Traceroute".to_owned(),
            Self::CheckCertificate { port } => format!("Check TLS certificate (port {port})"),
        }
    }

    pub fn group(&self) -> ActionGroup {
        match self {
            Self::OpenWeb { .. } | Self::BrowseFiles => ActionGroup::Open,
            Self::Ping | Self::Traceroute | Self::WakeOnLan | Self::CheckCertificate { .. } => {
                ActionGroup::Network
            }
            Self::CopySshCommand | Self::CopyAddress | Self::CopyMac | Self::CopyHostname => {
                ActionGroup::Copy
            }
        }
    }

    /// Port that makes this action likely to work, when the scan saw it open.
    pub fn matching_port(&self) -> Option<u16> {
        match self {
            Self::OpenWeb { port, .. } => Some(*port),
            Self::CheckCertificate { port } => Some(*port),
            Self::BrowseFiles => Some(445),
            Self::CopySshCommand => Some(22),
            _ => None,
        }
    }

    pub fn effect(&self, host: &Host) -> Option<Effect> {
        match self {
            Self::OpenWeb { https, port } => {
                let scheme = if *https { "https" } else { "http" };
                let default_port = if *https { 443 } else { 80 };
                let url = if *port == default_port {
                    format!("{scheme}://{}", host.ip)
                } else {
                    format!("{scheme}://{}:{port}", host.ip)
                };
                Some(Effect::Open(url))
            }
            Self::BrowseFiles => Some(Effect::Open(share_target(host.ip))),
            Self::CopySshCommand => Some(Effect::Copy(format!("ssh {}", host.ip))),
            Self::CopyAddress => Some(Effect::Copy(host.ip.to_string())),
            Self::CopyMac => host.mac.clone().map(Effect::Copy),
            Self::CopyHostname => host.hostname.clone().map(Effect::Copy),
            Self::WakeOnLan => host.mac.clone().map(Effect::Wake),
            Self::Ping => Some(Effect::Diagnose(Diagnostic::Ping, host.ip)),
            Self::Traceroute => Some(Effect::Diagnose(Diagnostic::Traceroute, host.ip)),
            Self::CheckCertificate { port } => {
                Some(Effect::CheckCertificate(format!("{}:{port}", host.ip)))
            }
        }
    }
}

/// The actions for `host` under their headings. Empty groups are left out.
pub fn grouped_actions(host: &Host) -> Vec<(ActionGroup, Vec<HostAction>)> {
    let actions = actions_for(host);
    ActionGroup::ALL
        .into_iter()
        .filter_map(|group| {
            let members: Vec<HostAction> = actions
                .iter()
                .filter(|action| action.group() == group)
                .cloned()
                .collect();
            (!members.is_empty()).then_some((group, members))
        })
        .collect()
}

/// Actions offered for `host`. Web ports beyond 80 and 443 appear only
/// when the scan found them open.
pub fn actions_for(host: &Host) -> Vec<HostAction> {
    let mut actions: Vec<HostAction> = WEB_PORTS
        .iter()
        .filter(|(port, _)| matches!(port, 80 | 443) || host.open_ports.contains(port))
        .map(|&(port, https)| HostAction::OpenWeb { https, port })
        .collect();
    actions.push(HostAction::BrowseFiles);
    actions.push(HostAction::CopySshCommand);
    actions.push(HostAction::Ping);
    actions.push(HostAction::Traceroute);
    actions.extend(
        TLS_PORTS
            .iter()
            .filter(|port| host.open_ports.contains(port))
            .map(|&port| HostAction::CheckCertificate { port }),
    );
    actions.push(HostAction::CopyAddress);
    if host.mac.is_some() {
        actions.push(HostAction::WakeOnLan);
        actions.push(HostAction::CopyMac);
    }
    if host.hostname.is_some() {
        actions.push(HostAction::CopyHostname);
    }
    actions
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host::new("192.168.0.9".parse().unwrap())
    }

    #[test]
    fn offers_default_web_ports_and_copy_actions() {
        let actions = actions_for(&host());
        assert!(actions.contains(&HostAction::OpenWeb {
            https: false,
            port: 80
        }));
        assert!(actions.contains(&HostAction::OpenWeb {
            https: true,
            port: 443
        }));
        assert!(!actions.contains(&HostAction::CopyMac));
        assert!(!actions.contains(&HostAction::CopyHostname));
    }

    #[test]
    fn ping_and_traceroute_target_the_host_address() {
        let found = host();
        assert!(actions_for(&found).contains(&HostAction::Ping));
        assert_eq!(
            HostAction::Traceroute.effect(&found),
            Some(Effect::Diagnose(Diagnostic::Traceroute, found.ip))
        );
    }

    #[test]
    fn groups_every_action_once_in_menu_order() {
        let mut found = host();
        found.mac = Some("02:00:5E:10:00:01".into());
        found.hostname = Some("nas.lan".into());
        let groups = grouped_actions(&found);
        let titles: Vec<&str> = groups.iter().map(|(group, _)| group.title()).collect();
        assert_eq!(titles, ["Open", "Network", "Copy"]);
        let grouped: usize = groups.iter().map(|(_, members)| members.len()).sum();
        assert_eq!(grouped, actions_for(&found).len());
        assert!(groups[1].1.contains(&HostAction::WakeOnLan));
        assert!(groups[2].1.contains(&HostAction::CopySshCommand));
    }

    #[test]
    fn adds_alternate_web_ports_only_when_open() {
        let mut found = host();
        found.open_ports = vec![8080];
        assert!(actions_for(&found).contains(&HostAction::OpenWeb {
            https: false,
            port: 8080
        }));
        assert!(!actions_for(&host()).contains(&HostAction::OpenWeb {
            https: false,
            port: 8080
        }));
    }

    #[test]
    fn builds_urls_without_default_port() {
        let web = HostAction::OpenWeb {
            https: true,
            port: 443,
        };
        assert_eq!(
            web.effect(&host()),
            Some(Effect::Open("https://192.168.0.9".into()))
        );
        let alt = HostAction::OpenWeb {
            https: false,
            port: 8080,
        };
        assert_eq!(
            alt.effect(&host()),
            Some(Effect::Open("http://192.168.0.9:8080".into()))
        );
    }

    #[test]
    fn offers_a_certificate_check_only_for_open_tls_ports() {
        let found = host();
        assert!(!actions_for(&found).contains(&HostAction::CheckCertificate { port: 443 }));
        let mut with_tls = host();
        with_tls.open_ports = vec![443, 8080];
        let actions = actions_for(&with_tls);
        assert!(actions.contains(&HostAction::CheckCertificate { port: 443 }));
        assert!(!actions.contains(&HostAction::CheckCertificate { port: 8443 }));
        assert_eq!(
            HostAction::CheckCertificate { port: 443 }.effect(&with_tls),
            Some(Effect::CheckCertificate("192.168.0.9:443".into()))
        );
    }

    #[test]
    fn wake_needs_a_mac_address() {
        assert!(!actions_for(&host()).contains(&HostAction::WakeOnLan));
        let mut found = host();
        found.mac = Some("AA:BB:CC:DD:EE:FF".into());
        assert!(actions_for(&found).contains(&HostAction::WakeOnLan));
        assert_eq!(
            HostAction::WakeOnLan.effect(&found),
            Some(Effect::Wake("AA:BB:CC:DD:EE:FF".into()))
        );
    }

    #[test]
    fn copy_actions_use_host_details() {
        let mut found = host();
        found.mac = Some("AA:BB:CC:DD:EE:FF".into());
        assert_eq!(
            HostAction::CopySshCommand.effect(&found),
            Some(Effect::Copy("ssh 192.168.0.9".into()))
        );
        assert_eq!(
            HostAction::CopyMac.effect(&found),
            Some(Effect::Copy("AA:BB:CC:DD:EE:FF".into()))
        );
        assert_eq!(HostAction::CopyHostname.effect(&found), None);
    }
}
