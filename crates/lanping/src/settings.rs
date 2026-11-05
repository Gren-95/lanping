//! User preferences: which ports to scan and which columns to show.

use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::path::Path;

use lanping_core::ports::{PortListError, common_ports, parse_ports};
use serde::{Deserialize, Serialize};

use crate::host_filter::HostFilter;
use crate::host_order::SortKey;
use crate::presets::FilterPreset;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Column {
    Address,
    Latency,
    Hostname,
    Mac,
    Vendor,
    Ports,
    DeviceType,
    /// The "Open with" button.
    Actions,
}

impl Column {
    pub const ALL: [Column; 8] = [
        Column::Address,
        Column::Latency,
        Column::Hostname,
        Column::Mac,
        Column::Vendor,
        Column::Ports,
        Column::DeviceType,
        Column::Actions,
    ];

    /// How the list sorts when this header is clicked; `None` for columns without an order.
    pub fn sort_key(self) -> Option<SortKey> {
        match self {
            Column::Address => Some(SortKey::Address),
            Column::Latency => Some(SortKey::Latency),
            Column::Hostname => Some(SortKey::Hostname),
            Column::Mac => Some(SortKey::Mac),
            Column::Vendor => Some(SortKey::Vendor),
            Column::Ports => Some(SortKey::Ports),
            Column::DeviceType => Some(SortKey::DeviceType),
            Column::Actions => None,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Column::Address => "Address",
            Column::Latency => "Latency",
            Column::Hostname => "Hostname",
            Column::Mac => "MAC address",
            Column::Vendor => "Vendor",
            Column::Ports => "Open ports",
            Column::DeviceType => "Type",
            Column::Actions => "Open with",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnSetting {
    pub column: Column,
    pub visible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SettingsTab {
    #[default]
    CommonPorts,
    CustomPorts,
    Columns,
    Monitoring,
    Network,
    Presets,
}

/// Fields missing from an older settings file take their default value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub selected_ports: BTreeSet<u16>,
    pub custom_ports: String,
    /// Read the greeting of every open port found.
    pub grab_banners: bool,
    /// SNMP community to read device details with. Empty leaves SNMP off.
    pub snmp_community: String,
    pub columns: Vec<ColumnSetting>,
    /// Minutes between automatic rescans; `0` turns them off.
    pub auto_rescan_minutes: u32,
    pub notify_new_devices: bool,
    /// Notify when a known device opens a port it did not have open before.
    pub notify_ports_opened: bool,
    /// Notify when a device the user named goes offline.
    pub notify_named_offline: bool,
    /// Hide container bridges, VPN tunnels and VM switches from the network picker.
    pub ignore_virtual_networks: bool,
    /// Show hosts as stacked cards instead of a table.
    pub card_view: bool,
    /// Start a scan as soon as the app opens.
    pub scan_on_startup: bool,
    /// Scan again when the main network changes, for example on a new Wi-Fi.
    pub scan_on_network_change: bool,
    pub filter_presets: Vec<FilterPreset>,
    /// Window size at the last close, in logical pixels.
    pub window_size: Option<(u32, u32)>,
    /// Filter that was active at the last close.
    pub last_filter: HostFilter,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            selected_ports: common_ports().collect(),
            custom_ports: String::new(),
            grab_banners: false,
            snmp_community: String::new(),
            auto_rescan_minutes: 0,
            notify_new_devices: true,
            notify_ports_opened: false,
            notify_named_offline: false,
            ignore_virtual_networks: true,
            card_view: false,
            scan_on_startup: false,
            scan_on_network_change: false,
            filter_presets: Vec::new(),
            window_size: None,
            last_filter: HostFilter::default(),
            columns: Column::ALL
                .iter()
                .map(|&column| ColumnSetting {
                    column,
                    visible: true,
                })
                .collect(),
        }
    }
}

impl Settings {
    pub fn select_all_ports(&mut self) {
        self.selected_ports = common_ports().collect();
    }

    pub fn select_no_ports(&mut self) {
        self.selected_ports.clear();
    }

    pub fn set_port_selected(&mut self, port: u16, selected: bool) {
        if selected {
            self.selected_ports.insert(port);
        } else {
            self.selected_ports.remove(&port);
        }
    }

    /// Ports to scan: ticked common ports plus the custom list.
    pub fn resolve_ports(&self) -> Result<Vec<u16>, PortListError> {
        let mut ports: BTreeSet<u16> = self.selected_ports.clone();
        ports.extend(parse_ports(&self.custom_ports)?);
        Ok(ports.into_iter().collect())
    }

    pub fn visible_columns(&self) -> Vec<Column> {
        self.columns
            .iter()
            .filter(|setting| setting.visible)
            .map(|setting| setting.column)
            .collect()
    }

    /// The address column stays visible so rows always have an identity.
    pub fn set_column_visible(&mut self, column: Column, visible: bool) {
        if column == Column::Address {
            return;
        }
        if let Some(setting) = self.columns.iter_mut().find(|s| s.column == column) {
            setting.visible = visible;
        }
    }

    /// Moves `column` to where `target` sits; the columns in between shift over.
    pub fn move_column_to(&mut self, column: Column, target: Column) {
        let position = |wanted: Column| {
            self.columns
                .iter()
                .position(|setting| setting.column == wanted)
        };
        let (Some(from), Some(to)) = (position(column), position(target)) else {
            return;
        };
        let moved = self.columns.remove(from);
        self.columns.insert(to, moved);
    }

    /// Drops duplicate columns and appends any the file did not mention.
    fn normalized(mut self) -> Self {
        let mut seen = Vec::new();
        self.columns.retain(|setting| {
            let first_time = !seen.contains(&setting.column);
            seen.push(setting.column);
            first_time
        });
        for column in Column::ALL {
            if !seen.contains(&column) {
                self.columns.push(ColumnSetting {
                    column,
                    visible: true,
                });
            }
        }
        if let Some(address) = self
            .columns
            .iter_mut()
            .find(|s| s.column == Column::Address)
        {
            address.visible = true;
        }
        self
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        let json = fs::read_to_string(path)?;
        let settings: Settings = serde_json::from_str(&json).map_err(io::Error::other)?;
        Ok(settings.normalized())
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(directory) = path.parent() {
            fs::create_dir_all(directory)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        fs::write(path, json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanping_core::ports::COMMON_SERVICES;

    #[test]
    fn defaults_select_every_common_port() {
        let settings = Settings::default();
        assert_eq!(settings.selected_ports.len(), COMMON_SERVICES.len());
        assert_eq!(settings.visible_columns().len(), Column::ALL.len());
    }

    #[test]
    fn virtual_adapters_are_ignored_by_default_and_in_older_files() {
        assert!(Settings::default().ignore_virtual_networks);
        let older: Settings = serde_json::from_str(r#"{"grab_banners":true}"#).unwrap();
        assert!(older.ignore_virtual_networks);
        assert!(!older.card_view);
        assert!(!older.scan_on_startup && !older.scan_on_network_change);
    }

    #[test]
    fn window_and_filter_are_remembered_and_default_for_older_files() {
        let older: Settings = serde_json::from_str(r#"{"grab_banners":true}"#).unwrap();
        assert_eq!(older.window_size, None);
        assert_eq!(older.last_filter, HostFilter::default());
        let settings = Settings {
            window_size: Some((1200, 800)),
            last_filter: HostFilter {
                text: "nas".into(),
                ..HostFilter::default()
            },
            ..Settings::default()
        };
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
    }

    #[test]
    fn select_all_and_none() {
        let mut settings = Settings::default();
        settings.select_no_ports();
        assert!(settings.selected_ports.is_empty());
        settings.select_all_ports();
        assert_eq!(settings.selected_ports.len(), COMMON_SERVICES.len());
    }

    #[test]
    fn resolve_merges_selected_and_custom_ports() {
        let mut settings = Settings::default();
        settings.select_no_ports();
        settings.set_port_selected(443, true);
        settings.custom_ports = "22, 8000-8001, 443".into();
        assert_eq!(settings.resolve_ports().unwrap(), vec![22, 443, 8000, 8001]);
    }

    #[test]
    fn resolve_reports_bad_custom_ports() {
        let settings = Settings {
            custom_ports: "http".into(),
            ..Settings::default()
        };
        assert!(settings.resolve_ports().is_err());
    }

    #[test]
    fn address_column_cannot_be_hidden() {
        let mut settings = Settings::default();
        settings.set_column_visible(Column::Address, false);
        settings.set_column_visible(Column::Vendor, false);
        let visible = settings.visible_columns();
        assert!(visible.contains(&Column::Address));
        assert!(!visible.contains(&Column::Vendor));
    }

    #[test]
    fn dragging_a_column_onto_another_takes_its_place() {
        let mut settings = Settings::default();
        settings.move_column_to(Column::Vendor, Column::Latency);
        assert_eq!(
            &settings.visible_columns()[..3],
            &[Column::Address, Column::Vendor, Column::Latency]
        );
        settings.move_column_to(Column::Address, Column::Actions);
        assert_eq!(
            settings.columns.last().map(|s| s.column),
            Some(Column::Address)
        );
        settings.move_column_to(Column::Address, Column::Address);
        assert_eq!(
            settings.columns.last().map(|s| s.column),
            Some(Column::Address)
        );
    }

    #[test]
    fn load_repairs_missing_and_duplicate_columns() {
        let broken = Settings {
            columns: vec![
                ColumnSetting {
                    column: Column::Vendor,
                    visible: true,
                },
                ColumnSetting {
                    column: Column::Vendor,
                    visible: false,
                },
                ColumnSetting {
                    column: Column::Address,
                    visible: false,
                },
            ],
            ..Settings::default()
        };
        let repaired = broken.normalized();
        assert_eq!(repaired.columns.len(), Column::ALL.len());
        assert!(repaired.visible_columns().contains(&Column::Address));
        assert_eq!(repaired.columns[0].column, Column::Vendor);
    }

    #[test]
    fn settings_from_an_older_version_still_load() {
        let old = r#"{"selected_ports":[22],"custom_ports":"","columns":[]}"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(settings.resolve_ports().unwrap(), [22]);
        assert_eq!(settings.auto_rescan_minutes, 0);
        assert!(settings.notify_new_devices);
    }

    #[test]
    fn saves_and_loads() {
        let directory =
            std::env::temp_dir().join(format!("lanping-settings-{}", std::process::id()));
        let path = directory.join("settings.json");
        let settings = Settings {
            grab_banners: true,
            custom_ports: "9000".into(),
            ..Settings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), settings);
        let _ = fs::remove_dir_all(&directory);
    }
}
