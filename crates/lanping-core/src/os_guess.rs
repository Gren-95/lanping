//! A best guess at a host's operating system, from what it reveals without logging in.

use crate::host::Host;

/// Words in a service banner that name an OS, most specific first.
const BANNER_RULES: [(&str, &str); 12] = [
    ("ubuntu", "Ubuntu Linux"),
    ("debian", "Debian Linux"),
    ("fedora", "Fedora Linux"),
    ("raspbian", "Raspberry Pi OS"),
    ("routeros", "MikroTik RouterOS"),
    ("openwrt", "OpenWrt"),
    ("synology", "Synology DSM"),
    ("freebsd", "FreeBSD"),
    ("microsoft", "Windows"),
    ("windows", "Windows"),
    ("darwin", "macOS"),
    ("linux", "Linux"),
];

/// Ports only one family normally opens. Ports shared through Samba or similar are left out.
const WINDOWS_PORTS: [u16; 2] = [135, 3389];
const WINDOWS_TTL: std::ops::RangeInclusive<u8> = 65..=128;
const UNIX_TTL: std::ops::RangeInclusive<u8> = 1..=64;
const MACOS_SERVICES: [&str; 2] = ["afpovertcp", "device-info"];

/// Banners count most, then ports and services, then the ping TTL.
/// Returns `None` when the evidence is too weak to say anything.
pub fn guess_os(host: &Host) -> Option<&'static str> {
    let banners = host
        .banners
        .iter()
        .map(|banner| banner.text.as_str())
        .chain(host.snmp.description.as_deref())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ");
    if let Some((_, os)) = BANNER_RULES.iter().find(|(word, _)| banners.contains(word)) {
        return Some(os);
    }
    if host
        .open_ports
        .iter()
        .any(|port| WINDOWS_PORTS.contains(port))
    {
        return Some("Windows");
    }
    let services = &host.discovery.services;
    if MACOS_SERVICES
        .iter()
        .any(|name| services.iter().any(|service| service == name))
    {
        return Some("macOS");
    }
    match host.ttl {
        Some(ttl) if WINDOWS_TTL.contains(&ttl) => Some("Windows"),
        Some(ttl) if UNIX_TTL.contains(&ttl) => Some("Linux, macOS or similar"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::ServiceBanner;

    fn host() -> Host {
        Host::new("10.0.0.5".parse().unwrap())
    }

    #[test]
    fn says_nothing_without_evidence() {
        let mut found = host();
        assert_eq!(guess_os(&found), None);
        found.open_ports = vec![80, 443];
        assert_eq!(guess_os(&found), None);
        found.ttl = Some(255);
        assert_eq!(guess_os(&found), None);
    }

    #[test]
    fn banners_beat_ports_and_ttl() {
        let mut found = host();
        found.ttl = Some(128);
        found.open_ports = vec![3389];
        found.banners = vec![ServiceBanner {
            port: 22,
            text: "SSH-2.0-OpenSSH_9.6p1 Ubuntu-3".into(),
        }];
        assert_eq!(guess_os(&found), Some("Ubuntu Linux"));
    }

    #[test]
    fn windows_ports_beat_ttl() {
        let mut found = host();
        found.ttl = Some(64);
        found.open_ports = vec![135];
        assert_eq!(guess_os(&found), Some("Windows"));
    }

    #[test]
    fn samba_ports_alone_do_not_mean_windows() {
        let mut found = host();
        found.open_ports = vec![139, 445];
        assert_eq!(guess_os(&found), None);
    }

    #[test]
    fn ttl_is_the_last_resort() {
        let with_ttl = |ttl| {
            let mut found = host();
            found.ttl = Some(ttl);
            guess_os(&found)
        };
        assert_eq!(with_ttl(64), Some("Linux, macOS or similar"));
        assert_eq!(with_ttl(120), Some("Windows"));
    }
}
