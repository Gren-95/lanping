//! A best guess at what kind of device a host is, from what the scan learned about it.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::host::Host;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum DeviceType {
    Network,
    Computer,
    Storage,
    Phone,
    Printer,
    Media,
    Camera,
    SmartHome,
    Unknown,
}

impl DeviceType {
    pub const ALL: [DeviceType; 9] = [
        DeviceType::Network,
        DeviceType::Computer,
        DeviceType::Storage,
        DeviceType::Phone,
        DeviceType::Printer,
        DeviceType::Media,
        DeviceType::Camera,
        DeviceType::SmartHome,
        DeviceType::Unknown,
    ];
}

impl fmt::Display for DeviceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Network => "Network gear",
            Self::Computer => "Computer",
            Self::Storage => "Storage",
            Self::Phone => "Phone or tablet",
            Self::Printer => "Printer",
            Self::Media => "TV or media",
            Self::Camera => "Camera",
            Self::SmartHome => "Smart home",
            Self::Unknown => "Unknown",
        })
    }
}

/// Words in a device's own name, model or service list. Whole words only, so
/// "tv" does not match inside longer words such as "activate".
const WORD_RULES: [(DeviceType, &[&str]); 8] = [
    (
        DeviceType::Printer,
        &[
            "printer",
            "ipp",
            "ipps",
            "pdl",
            "laserjet",
            "officejet",
            "deskjet",
        ],
    ),
    (
        DeviceType::Media,
        &[
            "tv",
            "googlecast",
            "chromecast",
            "airplay",
            "roku",
            "appletv",
            "shield",
            "soundbar",
            "sonos",
            "spotify",
        ],
    ),
    (
        DeviceType::Camera,
        &["camera", "cam", "ipcam", "doorbell", "nvr"],
    ),
    (
        DeviceType::Storage,
        &["nas", "diskstation", "synology", "qnap", "truenas"],
    ),
    (
        DeviceType::Phone,
        &[
            "iphone", "ipad", "android", "galaxy", "pixel", "phone", "tablet",
        ],
    ),
    (
        DeviceType::SmartHome,
        &[
            "tasmota",
            "shelly",
            "esp",
            "hap",
            "matter",
            "thermostat",
            "homeassistant",
            "hue",
            "plug",
            "bulb",
        ],
    ),
    (
        DeviceType::Network,
        &[
            "router", "gateway", "switch", "ap", "mikrotik", "openwrt", "unifi", "usw", "extender",
            "mesh",
        ],
    ),
    (
        DeviceType::Computer,
        &[
            "desktop",
            "laptop",
            "macbook",
            "imac",
            "pc",
            "workstation",
            "thinkpad",
            "ubuntu",
            "fedora",
        ],
    ),
];

/// Vendor names that mostly make one kind of device. Vendors that make many
/// kinds, such as Samsung or Apple, are left out on purpose.
const VENDOR_RULES: [(DeviceType, &[&str]); 7] = [
    (
        DeviceType::Network,
        &[
            "ubiquiti", "mikrotik", "tp-link", "netgear", "cisco", "zyxel", "aruba", "juniper",
        ],
    ),
    (
        DeviceType::Storage,
        &["synology", "qnap", "western digital"],
    ),
    (
        DeviceType::Printer,
        &["brother", "canon", "epson", "xerox", "lexmark", "kyocera"],
    ),
    (
        DeviceType::Camera,
        &["hikvision", "dahua", "axis", "reolink", "amcrest"],
    ),
    (
        DeviceType::SmartHome,
        &["espressif", "tuya", "shelly", "signify", "nest"],
    ),
    (DeviceType::Media, &["roku", "sonos", "vizio"]),
    (
        DeviceType::Computer,
        &[
            "intel",
            "micro-star",
            "asustek",
            "dell",
            "gigabyte",
            "lenovo",
            "raspberry",
        ],
    ),
];

const PRINTER_PORTS: [u16; 2] = [9100, 515];
const COMPUTER_PORTS: [u16; 4] = [3389, 445, 139, 135];
const DNS_PORT: u16 = 53;

fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
}

/// Guesses the type. The device's own words count most, then its vendor, then
/// its open ports. A device with no evidence stays `Unknown`.
pub fn guess_device_type(host: &Host) -> DeviceType {
    let own_words: Vec<String> = [
        host.hostname.as_deref(),
        host.discovery.name.as_deref(),
        host.discovery.model.as_deref(),
        host.snmp.name.as_deref(),
        host.snmp.model.as_deref(),
        host.snmp.description.as_deref(),
    ]
    .into_iter()
    .flatten()
    .flat_map(words)
    .chain(
        host.discovery
            .services
            .iter()
            .flat_map(|service| words(service)),
    )
    .collect();
    let by_word = WORD_RULES.iter().find(|(_, rule_words)| {
        own_words
            .iter()
            .any(|word| rule_words.contains(&word.as_str()))
    });
    if let Some((device_type, _)) = by_word {
        return *device_type;
    }

    let vendor = [
        host.vendor.as_deref(),
        host.discovery.manufacturer.as_deref(),
    ]
    .into_iter()
    .flatten()
    .map(str::to_lowercase)
    .collect::<Vec<_>>()
    .join(" ");
    let by_vendor = VENDOR_RULES
        .iter()
        .find(|(_, names)| names.iter().any(|name| vendor.contains(name)));
    if let Some((device_type, _)) = by_vendor {
        return *device_type;
    }

    let has_any = |ports: &[u16]| host.open_ports.iter().any(|port| ports.contains(port));
    if has_any(&PRINTER_PORTS) {
        DeviceType::Printer
    } else if has_any(&[DNS_PORT]) {
        DeviceType::Network
    } else if has_any(&COMPUTER_PORTS) {
        DeviceType::Computer
    } else {
        DeviceType::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> Host {
        Host::new("10.0.0.5".parse().unwrap())
    }

    #[test]
    fn with_no_evidence_it_stays_unknown() {
        assert_eq!(guess_device_type(&host()), DeviceType::Unknown);
    }

    #[test]
    fn names_match_whole_words_only() {
        let mut found = host();
        found.hostname = Some("living-room-tv.lan".into());
        assert_eq!(guess_device_type(&found), DeviceType::Media);
        found.hostname = Some("activate.lan".into());
        assert_eq!(guess_device_type(&found), DeviceType::Unknown);
    }

    #[test]
    fn services_and_models_count_as_the_devices_own_words() {
        let mut found = host();
        found.discovery.services = vec!["ipp".into()];
        assert_eq!(guess_device_type(&found), DeviceType::Printer);
        let mut camera = host();
        camera.discovery.model = Some("Doorbell 2".into());
        assert_eq!(guess_device_type(&camera), DeviceType::Camera);
    }

    #[test]
    fn own_words_beat_the_vendor_and_the_vendor_beats_ports() {
        let mut found = host();
        found.vendor = Some("Intel Corporate".into());
        found.open_ports = vec![9100];
        assert_eq!(guess_device_type(&found), DeviceType::Computer);
        found.hostname = Some("nas.lan".into());
        assert_eq!(guess_device_type(&found), DeviceType::Storage);
    }

    #[test]
    fn ports_are_the_last_resort() {
        let with_ports = |ports: &[u16]| {
            let mut found = host();
            found.open_ports = ports.to_vec();
            guess_device_type(&found)
        };
        assert_eq!(with_ports(&[9100]), DeviceType::Printer);
        assert_eq!(with_ports(&[53, 80]), DeviceType::Network);
        assert_eq!(with_ports(&[445]), DeviceType::Computer);
        assert_eq!(with_ports(&[80]), DeviceType::Unknown);
    }

    #[test]
    fn multi_purpose_vendors_alone_decide_nothing() {
        let mut found = host();
        found.vendor = Some("Samsung Electronics".into());
        assert_eq!(guess_device_type(&found), DeviceType::Unknown);
    }
}
