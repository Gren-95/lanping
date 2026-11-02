//! Automatic rescans and the new-device alert text.

use std::fmt;

use lanping_core::Host;
use lanping_core::diff::{HostStatus, ScanDiff};
use lanping_core::labels::DeviceLabels;
use lanping_core::ports::format_ports;

const MAX_NAMED_DEVICES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoInterval(pub u32);

impl AutoInterval {
    pub const ALL: [AutoInterval; 6] = [
        AutoInterval(0),
        AutoInterval(1),
        AutoInterval(5),
        AutoInterval(15),
        AutoInterval(30),
        AutoInterval(60),
    ];

    pub fn minutes(self) -> Option<u32> {
        (self.0 > 0).then_some(self.0)
    }
}

impl fmt::Display for AutoInterval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            0 => f.write_str("Off"),
            1 => f.write_str("Every minute"),
            minutes => write!(f, "Every {minutes} minutes"),
        }
    }
}

/// Body text for a "new devices" alert, or `None` when nothing is new.
fn new_device_alert(hosts: &[Host], diff: &ScanDiff, labels: &DeviceLabels) -> Option<String> {
    let new_hosts: Vec<&Host> = hosts
        .iter()
        .filter(|host| diff.status(host.ip) == HostStatus::New)
        .collect();
    if new_hosts.is_empty() {
        return None;
    }
    let names: Vec<String> = new_hosts
        .iter()
        .take(MAX_NAMED_DEVICES)
        .map(|host| describe(host, labels))
        .collect();
    let extra = new_hosts.len().saturating_sub(MAX_NAMED_DEVICES);
    let mut text = format!(
        "{} new {}: {}",
        new_hosts.len(),
        if new_hosts.len() == 1 {
            "device"
        } else {
            "devices"
        },
        names.join(", ")
    );
    if extra > 0 {
        text.push_str(&format!(" and {extra} more"));
    }
    Some(text)
}

/// Which changes between two scans raise a notification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlertRules {
    pub new_devices: bool,
    pub ports_opened: bool,
    pub named_offline: bool,
}

/// Body text for "port opened" alerts. A scan with no open ports says nothing,
/// since port scanning may have been off.
fn port_alert(previous: &[Host], hosts: &[Host], labels: &DeviceLabels) -> Option<String> {
    let mut lines: Vec<String> = hosts
        .iter()
        .filter_map(|host| {
            let key = host.device_key();
            let before = previous.iter().find(|old| old.device_key() == key)?;
            if before.open_ports.is_empty() {
                return None;
            }
            let opened: Vec<u16> = host
                .open_ports
                .iter()
                .copied()
                .filter(|port| !before.open_ports.contains(port))
                .collect();
            (!opened.is_empty()).then(|| {
                format!(
                    "{} opened port {}",
                    describe(host, labels),
                    format_ports(&opened)
                )
            })
        })
        .collect();
    let extra = lines.len().saturating_sub(MAX_NAMED_DEVICES);
    lines.truncate(MAX_NAMED_DEVICES);
    let mut text = (!lines.is_empty()).then(|| lines.join(", "))?;
    if extra > 0 {
        text.push_str(&format!(" and {extra} more"));
    }
    Some(text)
}

/// Body text for devices the user named that went offline.
fn offline_alert(diff: &ScanDiff, labels: &DeviceLabels) -> Option<String> {
    let names: Vec<&str> = diff
        .gone
        .iter()
        .filter_map(|host| labels.get(&host.device_key()))
        .collect();
    if names.is_empty() {
        return None;
    }
    let shown = names.len().min(MAX_NAMED_DEVICES);
    let mut text = format!("Offline: {}", names[..shown].join(", "));
    if names.len() > shown {
        text.push_str(&format!(" and {} more", names.len() - shown));
    }
    Some(text)
}

/// Everything worth a notification after a scan, or `None` when nothing is.
pub fn scan_alert(
    previous: &[Host],
    hosts: &[Host],
    diff: &ScanDiff,
    labels: &DeviceLabels,
    rules: AlertRules,
) -> Option<String> {
    let parts: Vec<String> = [
        rules
            .new_devices
            .then(|| new_device_alert(hosts, diff, labels))
            .flatten(),
        rules
            .ports_opened
            .then(|| port_alert(previous, hosts, labels))
            .flatten(),
        rules
            .named_offline
            .then(|| offline_alert(diff, labels))
            .flatten(),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(". "))
}

fn describe(host: &Host, labels: &DeviceLabels) -> String {
    let name = labels
        .get(&host.device_key())
        .map(str::to_owned)
        .or_else(|| host.display_name().map(str::to_owned))
        .or_else(|| host.vendor.clone());
    match name {
        Some(name) => format!("{name} ({})", host.ip),
        None => host.ip.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanping_core::diff::diff_scans;

    fn host(last_octet: u8) -> Host {
        Host::new(std::net::Ipv4Addr::new(10, 0, 0, last_octet))
    }

    #[test]
    fn labels_intervals() {
        assert_eq!(AutoInterval(0).to_string(), "Off");
        assert_eq!(AutoInterval(1).to_string(), "Every minute");
        assert_eq!(AutoInterval(15).to_string(), "Every 15 minutes");
        assert_eq!(AutoInterval(0).minutes(), None);
        assert_eq!(AutoInterval(5).minutes(), Some(5));
    }

    #[test]
    fn no_alert_without_new_devices() {
        let hosts = [host(1)];
        let diff = diff_scans(&hosts, &hosts);
        assert_eq!(
            new_device_alert(&hosts, &diff, &DeviceLabels::default()),
            None
        );
    }

    #[test]
    fn names_new_devices_and_caps_the_list() {
        let mut labels = DeviceLabels::default();
        labels.set("10.0.0.2", "Printer");
        let current: Vec<Host> = (2..=6).map(host).collect();
        let diff = diff_scans(&[host(1)], &current);
        let alert = new_device_alert(&current, &diff, &labels).unwrap();
        assert_eq!(
            alert,
            "5 new devices: Printer (10.0.0.2), 10.0.0.3, 10.0.0.4 and 2 more"
        );
    }

    const ALL_RULES: AlertRules = AlertRules {
        new_devices: true,
        ports_opened: true,
        named_offline: true,
    };

    fn with_ports(last_octet: u8, ports: &[u16]) -> Host {
        let mut found = host(last_octet);
        found.open_ports = ports.to_vec();
        found
    }

    #[test]
    fn alerts_on_newly_opened_ports_only() {
        let before = [with_ports(1, &[22])];
        let after = [with_ports(1, &[22, 80])];
        let diff = diff_scans(&before, &after);
        let labels = DeviceLabels::default();
        assert_eq!(
            scan_alert(&before, &after, &diff, &labels, ALL_RULES).as_deref(),
            Some("10.0.0.1 opened port 80")
        );
        assert_eq!(
            scan_alert(
                &after,
                &before,
                &diff_scans(&after, &before),
                &labels,
                ALL_RULES
            ),
            None
        );
    }

    #[test]
    fn a_previous_scan_without_ports_raises_no_port_alert() {
        let before = [with_ports(1, &[])];
        let after = [with_ports(1, &[22])];
        let diff = diff_scans(&before, &after);
        assert_eq!(
            scan_alert(&before, &after, &diff, &DeviceLabels::default(), ALL_RULES),
            None
        );
    }

    #[test]
    fn alerts_only_for_named_devices_going_offline() {
        let before = [host(1), host(2)];
        let after: [Host; 0] = [];
        let diff = diff_scans(&before, &after);
        let mut labels = DeviceLabels::default();
        assert_eq!(scan_alert(&before, &after, &diff, &labels, ALL_RULES), None);
        labels.set("10.0.0.2", "Printer");
        assert_eq!(
            scan_alert(&before, &after, &diff, &labels, ALL_RULES).as_deref(),
            Some("Offline: Printer")
        );
    }

    #[test]
    fn rules_that_are_off_stay_quiet_and_others_combine() {
        let before = [with_ports(1, &[22]), host(2)];
        let after = [with_ports(1, &[22, 80]), host(3)];
        let diff = diff_scans(&before, &after);
        let mut labels = DeviceLabels::default();
        labels.set("10.0.0.2", "Printer");
        assert_eq!(
            scan_alert(&before, &after, &diff, &labels, ALL_RULES).as_deref(),
            Some("1 new device: 10.0.0.3. 10.0.0.1 opened port 80. Offline: Printer")
        );
        let none = AlertRules {
            new_devices: false,
            ports_opened: false,
            named_offline: false,
        };
        assert_eq!(scan_alert(&before, &after, &diff, &labels, none), None);
    }

    #[test]
    fn singular_wording() {
        let current = [host(1), host(2)];
        let diff = diff_scans(&[host(1)], &current);
        assert_eq!(
            new_device_alert(&current, &diff, &DeviceLabels::default()).as_deref(),
            Some("1 new device: 10.0.0.2")
        );
    }
}
