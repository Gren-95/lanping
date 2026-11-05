use lanping_core::range::{LocalNetwork, NetworkKind};

use crate::settings::Settings;

/// The target of the first listed network in `new` when it differs from the first in `old`.
pub(super) fn changed_primary_target(old: &[LocalNetwork], new: &[LocalNetwork]) -> Option<String> {
    let primary = new.first()?;
    (old.first() != Some(primary)).then(|| primary.target.clone())
}

/// Detected networks the picker offers, honoring the virtual adapter setting.
pub(super) fn listed_networks(networks: &[LocalNetwork], settings: &Settings) -> Vec<LocalNetwork> {
    networks
        .iter()
        .filter(|network| {
            !(settings.ignore_virtual_networks && network.kind == NetworkKind::Virtual)
        })
        .cloned()
        .collect()
}

/// Networks whose range or interface contains `typed`. All of them for an empty field;
/// none once the field holds exactly one of their targets.
pub(super) fn matching_networks(networks: Vec<LocalNetwork>, typed: &str) -> Vec<LocalNetwork> {
    let typed = typed.trim().to_lowercase();
    if networks
        .iter()
        .any(|network| network.target.to_lowercase() == typed)
    {
        return Vec::new();
    }
    networks
        .into_iter()
        .filter(|network| {
            network.target.to_lowercase().contains(&typed)
                || network.interface.to_lowercase().contains(&typed)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network(interface: &str, kind: NetworkKind) -> LocalNetwork {
        LocalNetwork {
            interface: interface.to_owned(),
            target: format!("10.0.{}.0/24", interface.len()),
            kind,
            host_count: 254,
        }
    }

    #[test]
    fn virtual_networks_are_listed_only_when_not_ignored() {
        let detected = [
            network("eth0", NetworkKind::Physical),
            network("docker0", NetworkKind::Virtual),
            network("Tailscale", NetworkKind::Tailnet),
        ];
        let mut settings = Settings::default();
        let names = |networks: Vec<LocalNetwork>| -> Vec<String> {
            networks
                .into_iter()
                .map(|network| network.interface)
                .collect()
        };
        assert_eq!(
            names(listed_networks(&detected, &settings)),
            ["eth0", "Tailscale"]
        );
        settings.ignore_virtual_networks = false;
        assert_eq!(
            names(listed_networks(&detected, &settings)),
            ["eth0", "docker0", "Tailscale"]
        );
    }

    #[test]
    fn a_new_main_network_is_reported_once() {
        let home = network("eth0", NetworkKind::Physical);
        let cafe = network("wlan0", NetworkKind::Physical);
        let at_home = std::slice::from_ref(&home);
        assert_eq!(changed_primary_target(at_home, at_home), None);
        assert_eq!(
            changed_primary_target(at_home, &[cafe.clone(), home.clone()]),
            Some(cafe.target.clone())
        );
        assert_eq!(
            changed_primary_target(&[], at_home),
            Some(home.target.clone())
        );
        assert_eq!(changed_primary_target(&[cafe], &[]), None);
    }

    #[test]
    fn suggestions_follow_the_typed_text() {
        let detected = vec![
            network("eth0", NetworkKind::Physical),
            network("wlan", NetworkKind::Physical),
        ];
        let names = |typed: &str| -> Vec<String> {
            matching_networks(detected.clone(), typed)
                .into_iter()
                .map(|n| n.interface)
                .collect()
        };
        assert_eq!(names(""), ["eth0", "wlan"]);
        assert_eq!(names("10.0"), ["eth0", "wlan"]);
        assert_eq!(names("WLA"), ["wlan"]);
        assert!(names("172.16.0.7").is_empty());
        assert!(names(&detected[0].target).is_empty());
    }
}
