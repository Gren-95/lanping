//! Combines mDNS, NetBIOS and SSDP answers into what is known about a device.
//! Everything here comes from the network, so text is cleaned before it is kept.

use std::net::Ipv4Addr;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::mdns;
use crate::netbios;
use crate::ssdp::DeviceDescription;

const MAX_TEXT_CHARS: usize = 128;
const MAX_SERVICES: usize = 32;

/// What a device says about itself through discovery protocols.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Discovery {
    pub name: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
    /// Advertised mDNS service types, such as `ipp` or `http`.
    pub services: Vec<String>,
}

/// Drops control characters and limits the length. Empty text becomes `None`.
fn clean_text(raw: Option<String>) -> Option<String> {
    let cleaned: String = raw?
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_TEXT_CHARS)
        .collect();
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_owned())
}

/// Combines the answers. The name prefers mDNS, then NetBIOS, then the UPnP friendly name.
fn merge(
    mdns_name: Option<String>,
    netbios_name: Option<String>,
    services: Vec<String>,
) -> Discovery {
    Discovery {
        name: clean_text(mdns_name.or(netbios_name)),
        manufacturer: None,
        model: None,
        services: services
            .into_iter()
            .filter_map(|service| clean_text(Some(service)))
            .take(MAX_SERVICES)
            .collect(),
    }
}

impl Discovery {
    /// Adds what a UPnP description says, keeping any name already known.
    pub fn add_description(&mut self, description: DeviceDescription) {
        self.name = self.name.take().or(clean_text(description.friendly_name));
        self.manufacturer = clean_text(description.manufacturer);
        self.model = clean_text(description.model);
    }
}

/// Asks one host directly over mDNS and NetBIOS.
pub async fn query_host(ip: Ipv4Addr, wait: Duration) -> Discovery {
    let (mdns_name, services, netbios_name) = tokio::join!(
        mdns::query_name(ip, wait),
        mdns::query_services(ip, wait),
        netbios::query_name(ip, wait),
    );
    merge(mdns_name, netbios_name, services)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_mdns_names_and_keeps_a_known_name_over_upnp() {
        let mut found = merge(
            Some("nas".into()),
            Some("NAS-SMB".into()),
            vec!["smb".into()],
        );
        assert_eq!(found.name.as_deref(), Some("nas"));
        found.add_description(DeviceDescription {
            friendly_name: Some("Friendly".into()),
            manufacturer: Some("Maker".into()),
            model: Some("M1".into()),
        });
        assert_eq!(found.name.as_deref(), Some("nas"));
        assert_eq!(found.model.as_deref(), Some("M1"));
        let mut nameless = Discovery::default();
        nameless.add_description(DeviceDescription {
            friendly_name: Some("Friendly".into()),
            ..DeviceDescription::default()
        });
        assert_eq!(nameless.name.as_deref(), Some("Friendly"));
    }

    #[test]
    fn cleans_untrusted_text() {
        assert_eq!(
            clean_text(Some("  a\u{1b}[31mb\n ".into())).as_deref(),
            Some("a[31mb")
        );
        assert_eq!(clean_text(Some("\n\t".into())), None);
        assert_eq!(
            clean_text(Some("x".repeat(500))).unwrap().len(),
            MAX_TEXT_CHARS
        );
    }
}
