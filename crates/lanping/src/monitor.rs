//! Automatic rescans and the new-device alert text.

use std::fmt;

use lanping_core::Host;
use lanping_core::diff::{HostStatus, ScanDiff};
use lanping_core::labels::DeviceLabels;

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
pub fn new_device_alert(hosts: &[Host], diff: &ScanDiff, labels: &DeviceLabels) -> Option<String> {
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
