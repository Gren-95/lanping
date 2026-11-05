//! Result filters: free text plus latency, status, vendor, port and name conditions.

use std::fmt;
use std::time::Duration;

use lanping_core::Host;
use lanping_core::device_notes::DeviceNote;
use lanping_core::device_type::{DeviceType, guess_device_type};
use lanping_core::diff::HostStatus;
use lanping_core::ports::format_ports;
use lanping_core::text::contains_ignore_case;
use serde::{Deserialize, Serialize};

pub const FAST_LATENCY_MS: f64 = 40.0;
pub const SLOW_LATENCY_MS: f64 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum LatencyBand {
    #[default]
    Any,
    Fast,
    Medium,
    Slow,
    /// Hosts found only through the ARP cache.
    NoReply,
}

impl LatencyBand {
    pub const ALL: [LatencyBand; 5] = [
        LatencyBand::Any,
        LatencyBand::Fast,
        LatencyBand::Medium,
        LatencyBand::Slow,
        LatencyBand::NoReply,
    ];

    fn contains(self, latency: Option<Duration>) -> bool {
        let milliseconds = latency.map(|latency| latency.as_secs_f64() * 1000.0);
        match (self, milliseconds) {
            (Self::Any, _) => true,
            (Self::NoReply, None) => true,
            (Self::NoReply, Some(_)) | (_, None) => false,
            (Self::Fast, Some(ms)) => ms < FAST_LATENCY_MS,
            (Self::Medium, Some(ms)) => (FAST_LATENCY_MS..SLOW_LATENCY_MS).contains(&ms),
            (Self::Slow, Some(ms)) => ms >= SLOW_LATENCY_MS,
        }
    }
}

impl fmt::Display for LatencyBand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Any => "Any latency",
            Self::Fast => "Fast (under 40 ms)",
            Self::Medium => "Medium (40-150 ms)",
            Self::Slow => "Slow (over 150 ms)",
            Self::NoReply => "No reply (ARP only)",
        })
    }
}

/// How a listed device compares with the previous scan of the same network.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RowStatus {
    Unchanged,
    New,
    Changed,
    /// Seen in the previous scan, silent now.
    Gone,
}

impl RowStatus {
    pub const FILTERABLE: [RowStatus; 4] = [
        RowStatus::New,
        RowStatus::Changed,
        RowStatus::Gone,
        RowStatus::Unchanged,
    ];
}

impl From<HostStatus> for RowStatus {
    fn from(status: HostStatus) -> Self {
        match status {
            HostStatus::Unchanged => Self::Unchanged,
            HostStatus::New => Self::New,
            HostStatus::Changed => Self::Changed,
        }
    }
}

impl fmt::Display for Choice<RowStatus> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Any => "Any status",
            Self::Only(RowStatus::New) => "New",
            Self::Only(RowStatus::Changed) => "Changed",
            Self::Only(RowStatus::Gone) => "Gone",
            Self::Only(RowStatus::Unchanged) => "Unchanged",
        })
    }
}

/// A dropdown value that is either "everything" or one specific value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice<T> {
    Any,
    Only(T),
}

impl<T> From<Option<T>> for Choice<T> {
    fn from(value: Option<T>) -> Self {
        value.map_or(Self::Any, Self::Only)
    }
}

impl<T> Choice<T> {
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Any => None,
            Self::Only(value) => Some(value),
        }
    }
}

impl fmt::Display for Choice<String> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.write_str("Any vendor"),
            Self::Only(vendor) => f.write_str(vendor),
        }
    }
}

impl fmt::Display for Choice<DeviceType> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.write_str("Any type"),
            Self::Only(device_type) => write!(f, "{device_type}"),
        }
    }
}

impl fmt::Display for Choice<u16> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.write_str("Any open port"),
            Self::Only(port) => write!(f, "Port {port} open"),
        }
    }
}

/// Condition on a device's hostname and on the name the user gave it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NameFilter {
    #[default]
    Any,
    /// The network reports a hostname.
    HasHostname,
    /// The user gave the device a name.
    HasLabel,
    /// The user has not named the device.
    NoLabel,
}

impl NameFilter {
    pub const ALL: [NameFilter; 4] = [
        NameFilter::Any,
        NameFilter::HasHostname,
        NameFilter::HasLabel,
        NameFilter::NoLabel,
    ];

    fn contains(self, host: &Host, label: Option<&str>) -> bool {
        match self {
            Self::Any => true,
            Self::HasHostname => host.display_name().is_some(),
            Self::HasLabel => label.is_some(),
            Self::NoLabel => label.is_none(),
        }
    }
}

impl fmt::Display for NameFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Any => "Any name",
            Self::HasHostname => "Has hostname",
            Self::HasLabel => "Has your label",
            Self::NoLabel => "No label yet",
        })
    }
}

/// One edit to the filter, sent by the filter bar.
#[derive(Debug, Clone)]
pub enum FilterChange {
    Text(String),
    Latency(LatencyBand),
    Status(Choice<RowStatus>),
    Vendor(Choice<String>),
    Port(Choice<u16>),
    Name(NameFilter),
    DeviceType(Choice<DeviceType>),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HostFilter {
    pub text: String,
    pub latency: LatencyBand,
    pub vendor: Option<String>,
    pub port: Option<u16>,
    pub status: Option<RowStatus>,
    pub name: NameFilter,
    pub device_type: Option<DeviceType>,
}

impl HostFilter {
    pub fn apply(&mut self, change: FilterChange) {
        match change {
            FilterChange::Text(text) => self.text = text,
            FilterChange::Latency(band) => self.latency = band,
            FilterChange::Status(choice) => self.status = choice.value().copied(),
            FilterChange::Vendor(choice) => self.vendor = choice.value().cloned(),
            FilterChange::Port(choice) => self.port = choice.value().copied(),
            FilterChange::Name(name) => self.name = name,
            FilterChange::DeviceType(choice) => self.device_type = choice.value().copied(),
            FilterChange::Clear => *self = Self::default(),
        }
    }

    /// Number of conditions currently narrowing the list.
    pub fn active_count(&self) -> usize {
        [
            !self.text.trim().is_empty(),
            self.latency != LatencyBand::Any,
            self.vendor.is_some(),
            self.port.is_some(),
            self.status.is_some(),
            self.name != NameFilter::Any,
            self.device_type.is_some(),
        ]
        .into_iter()
        .filter(|active| *active)
        .count()
    }

    /// The search text as it is compared: trimmed and lowercase. Make it once for a whole list
    /// and pass it to [`HostFilter::matches_with`].
    pub fn needle(&self) -> String {
        self.text.trim().to_lowercase()
    }

    /// [`HostFilter::matches_with`] for one host, making the needle itself.
    #[cfg(test)]
    pub fn matches(
        &self,
        host: &Host,
        label: Option<&str>,
        note: Option<&DeviceNote>,
        status: RowStatus,
    ) -> bool {
        self.matches_with(host, label, note, status, &self.needle())
    }

    /// [`HostFilter::matches`] with the needle made beforehand.
    pub fn matches_with(
        &self,
        host: &Host,
        label: Option<&str>,
        note: Option<&DeviceNote>,
        status: RowStatus,
        needle: &str,
    ) -> bool {
        self.latency.contains(host.latency)
            && self.status.is_none_or(|wanted| wanted == status)
            && self.name.contains(host, label)
            && self
                .vendor
                .as_ref()
                .is_none_or(|vendor| host.vendor.as_ref() == Some(vendor))
            && self.port.is_none_or(|port| host.open_ports.contains(&port))
            && self
                .device_type
                .is_none_or(|wanted| guess_device_type(host) == wanted)
            && text_matches(host, label, note, needle)
    }
}

fn text_matches(host: &Host, label: Option<&str>, note: Option<&DeviceNote>, needle: &str) -> bool {
    if needle.is_empty() || note.is_some_and(|note| note.contains(needle)) {
        return true;
    }
    let found = |field: Option<&str>| field.is_some_and(|text| contains_ignore_case(text, needle));
    let snmp = &host.snmp;
    let discovery = &host.discovery;
    found(label)
        || found(host.hostname.as_deref())
        || found(discovery.name.as_deref())
        || found(snmp.name.as_deref())
        || found(snmp.description.as_deref())
        || found(snmp.model.as_deref())
        || found(snmp.serial.as_deref())
        || found(snmp.location.as_deref())
        || found(discovery.manufacturer.as_deref())
        || found(discovery.model.as_deref())
        || found(host.mac.as_deref())
        || found(host.vendor.as_deref())
        || discovery
            .services
            .iter()
            .any(|service| found(Some(service)))
        || found(Some(&host.ip.to_string()))
        || found(Some(&format_ports(&host.open_ports)))
        || host
            .ipv6
            .iter()
            .any(|address| found(Some(&address.to_string())))
}

/// Distinct vendors in `hosts`, sorted, for the vendor dropdown.
pub fn vendor_choices(hosts: &[Host]) -> Vec<Choice<String>> {
    let mut vendors: Vec<&String> = hosts
        .iter()
        .filter_map(|host| host.vendor.as_ref())
        .collect();
    // Equal names that differ only in case sit next to each other, so every copy is found.
    vendors.sort_by_cached_key(|vendor| (vendor.to_lowercase(), vendor.to_string()));
    vendors.dedup();
    std::iter::once(Choice::Any)
        .chain(vendors.into_iter().cloned().map(Choice::Only))
        .collect()
}

/// Distinct open ports in `hosts`, ascending, for the port dropdown.
pub fn port_choices(hosts: &[Host]) -> Vec<Choice<u16>> {
    let mut ports: Vec<u16> = hosts
        .iter()
        .flat_map(|host| host.open_ports.iter().copied())
        .collect();
    ports.sort_unstable();
    ports.dedup();
    std::iter::once(Choice::Any)
        .chain(ports.into_iter().map(Choice::Only))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(last_octet: u8) -> Host {
        Host::new(std::net::Ipv4Addr::new(10, 0, 0, last_octet))
    }

    fn with_latency(last_octet: u8, milliseconds: u64) -> Host {
        let mut found = host(last_octet);
        found.latency = Some(Duration::from_millis(milliseconds));
        found
    }

    #[test]
    fn empty_filter_matches_everything() {
        let filter = HostFilter::default();
        assert_eq!(filter.active_count(), 0);
        assert!(filter.matches(&host(1), None, None, RowStatus::Unchanged));
    }

    #[test]
    fn text_matches_any_field_case_insensitively() {
        let mut printer = host(5);
        printer.vendor = Some("Hewlett Packard".into());
        printer.open_ports = vec![9100];
        let text = |query: &str| HostFilter {
            text: query.into(),
            ..HostFilter::default()
        };
        assert!(text("HEWLETT").matches(&printer, None, None, RowStatus::Unchanged));
        assert!(text("9100").matches(&printer, None, None, RowStatus::Unchanged));
        assert!(text("10.0.0.5").matches(&printer, None, None, RowStatus::Unchanged));
        printer.ipv6 = vec!["fe80::5".parse().unwrap()];
        assert!(text("FE80::5").matches(&printer, None, None, RowStatus::Unchanged));
        assert!(!text("cisco").matches(&printer, None, None, RowStatus::Unchanged));
    }

    #[test]
    fn search_also_covers_the_users_notes() {
        let note = DeviceNote::from_input("Sam", "", "", "");
        let wanted = HostFilter {
            text: "SAM".into(),
            ..HostFilter::default()
        };
        assert!(wanted.matches(&host(1), None, Some(&note), RowStatus::Unchanged));
        assert!(!wanted.matches(&host(1), None, None, RowStatus::Unchanged));
    }

    #[test]
    fn filters_by_the_guessed_device_type() {
        let mut printer = host(5);
        printer.open_ports = vec![9100];
        let wanted = HostFilter {
            device_type: Some(DeviceType::Printer),
            ..HostFilter::default()
        };
        assert_eq!(wanted.active_count(), 1);
        assert!(wanted.matches(&printer, None, None, RowStatus::Unchanged));
        assert!(!wanted.matches(&host(6), None, None, RowStatus::Unchanged));
    }

    #[test]
    fn latency_bands_split_at_thresholds() {
        let band = |band| HostFilter {
            latency: band,
            ..HostFilter::default()
        };
        assert!(band(LatencyBand::Fast).matches(
            &with_latency(1, 39),
            None,
            None,
            RowStatus::Unchanged
        ));
        assert!(!band(LatencyBand::Fast).matches(
            &with_latency(1, 40),
            None,
            None,
            RowStatus::Unchanged
        ));
        assert!(band(LatencyBand::Medium).matches(
            &with_latency(1, 40),
            None,
            None,
            RowStatus::Unchanged
        ));
        assert!(band(LatencyBand::Medium).matches(
            &with_latency(1, 149),
            None,
            None,
            RowStatus::Unchanged
        ));
        assert!(band(LatencyBand::Slow).matches(
            &with_latency(1, 150),
            None,
            None,
            RowStatus::Unchanged
        ));
        assert!(band(LatencyBand::NoReply).matches(&host(1), None, None, RowStatus::Unchanged));
        assert!(!band(LatencyBand::NoReply).matches(
            &with_latency(1, 5),
            None,
            None,
            RowStatus::Unchanged
        ));
        assert!(!band(LatencyBand::Fast).matches(&host(1), None, None, RowStatus::Unchanged));
    }

    #[test]
    fn vendor_port_and_name_conditions_combine() {
        let mut nas = host(9);
        nas.vendor = Some("Synology".into());
        nas.open_ports = vec![22, 445];
        nas.hostname = Some("nas".into());
        let filter = HostFilter {
            vendor: Some("Synology".into()),
            port: Some(445),
            name: NameFilter::HasHostname,
            ..HostFilter::default()
        };
        assert_eq!(filter.active_count(), 3);
        assert!(filter.matches(&nas, None, None, RowStatus::Unchanged));
        nas.hostname = None;
        assert!(!filter.matches(&nas, None, None, RowStatus::Unchanged));
        assert!(
            !HostFilter {
                port: Some(80),
                ..HostFilter::default()
            }
            .matches(&nas, None, None, RowStatus::Unchanged)
        );
    }

    #[test]
    fn status_label_and_unlabeled_conditions() {
        let device = host(4);
        let new_only = HostFilter {
            status: Some(RowStatus::New),
            ..HostFilter::default()
        };
        assert!(new_only.matches(&device, None, None, RowStatus::New));
        assert!(!new_only.matches(&device, None, None, RowStatus::Unchanged));
        let unlabeled = HostFilter {
            name: NameFilter::NoLabel,
            ..HostFilter::default()
        };
        assert!(unlabeled.matches(&device, None, None, RowStatus::Unchanged));
        assert!(!unlabeled.matches(&device, Some("TV"), None, RowStatus::Unchanged));
        let labeled = HostFilter {
            name: NameFilter::HasLabel,
            ..HostFilter::default()
        };
        assert!(labeled.matches(&device, Some("TV"), None, RowStatus::Unchanged));
        assert!(!labeled.matches(&device, None, None, RowStatus::Unchanged));
        let by_label = HostFilter {
            text: "tv".into(),
            ..HostFilter::default()
        };
        assert!(by_label.matches(&device, Some("Living room TV"), None, RowStatus::Unchanged));
    }

    #[test]
    fn missing_saved_fields_use_defaults() {
        let filter: HostFilter = serde_json::from_str(r#"{"text":"nas"}"#).unwrap();
        assert_eq!(filter.text, "nas");
        assert_eq!(filter.name, NameFilter::Any);
        assert_eq!(filter.status, None);
    }

    #[test]
    fn changes_edit_one_condition_and_clear_resets_all() {
        let mut filter = HostFilter::default();
        filter.apply(FilterChange::Port(Choice::Only(22)));
        filter.apply(FilterChange::Name(NameFilter::NoLabel));
        assert_eq!(filter.active_count(), 2);
        filter.apply(FilterChange::Port(Choice::Any));
        assert_eq!(filter.port, None);
        assert_eq!(filter.active_count(), 1);
        filter.apply(FilterChange::Clear);
        assert_eq!(filter, HostFilter::default());
    }

    #[test]
    fn builds_sorted_distinct_choices() {
        let mut a = host(1);
        a.vendor = Some("intel".into());
        a.open_ports = vec![443, 22];
        let mut b = host(2);
        b.vendor = Some("Apple".into());
        b.open_ports = vec![22];
        let mut c = host(3);
        c.vendor = Some("intel".into());
        let hosts = [a, b, c];
        let vendors: Vec<String> = vendor_choices(&hosts)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(vendors, ["Any vendor", "Apple", "intel"]);
        let ports: Vec<String> = port_choices(&hosts)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(ports, ["Any open port", "Port 22 open", "Port 443 open"]);
    }
}
