use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use iced::task::Handle;
use iced::{Subscription, Task, Theme};
use lanping_core::diagnostics::run as run_diagnostic;
use lanping_core::diff::{ScanDiff, diff_scans};
use lanping_core::history::{SavedScan, Sighting, list_scans, load_scan, save_scan, sightings};
use lanping_core::labels::DeviceLabels;
use lanping_core::range::{LocalNetwork, NetworkKind, detect_local_networks};
use lanping_core::wol::send_wake_packet;
use lanping_core::{Host, ScanEvent, ScanStage};

use crate::actions::{Effect, HostAction};
use crate::copy_text::rows_as_text;
use crate::export_format::{ExportFormat, ask_save_path};
use crate::host_filter::{FilterChange, HostFilter, RowStatus};
use crate::host_order::{SortKey, compare_hosts};
use crate::host_rows::Row;
use crate::monitor::AutoInterval;
use crate::opener::open_target;
use crate::presets::{PresetChoice, upsert_preset};
use crate::settings::{Column, Settings, SettingsTab};
use crate::storage::{
    export_directory, history_directory, labels_path, migrate_legacy_data, settings_path,
};
use crate::timestamp::format_unix_utc;

/// How often the interface list is read while "scan when the network changes" is on.
const NETWORK_CHECK_INTERVAL: Duration = Duration::from_secs(10);
const FALLBACK_TARGET: &str = "192.168.0.0/24";
/// Widget id of the results search box, so Ctrl+F can focus it.
pub const SEARCH_INPUT_ID: &str = "search";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub path: PathBuf,
    pub target: String,
    pub label: String,
}

impl fmt::Display for HistoryEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    Settings(SettingsTab),
    /// Menu for one device, identified by [`Host::device_key`].
    HostActions(String),
    SavePreset,
    /// Everything known about one device, identified by [`Host::device_key`].
    HostDetails(String),
    /// Output of a ping or traceroute; `output` is empty while it runs.
    Diagnostic {
        title: String,
        output: Option<String>,
    },
    /// Move or hide one table column.
    ColumnMenu(Column),
}

#[derive(Debug, Clone)]
pub enum Message {
    TargetChanged(String),
    NetworkPicked(LocalNetwork),
    StartScan,
    StopScan,
    AutoRescanTick,
    Scan(ScanEvent),
    SortBy(SortKey),
    SortKeyPicked(SortKey),
    SortDirectionToggled,
    ExportPicked(ExportFormat),
    ExportPathChosen(ExportFormat, Option<PathBuf>),
    HistoryPicked(HistoryEntry),
    Filter(FilterChange),
    FiltersPanelToggled,
    FocusSearch,
    ShowHostDetails(String),
    DiagnosticFinished(String, Result<String, String>),
    ScanOnStartupToggled(bool),
    ScanOnNetworkChangeToggled(bool),
    CheckNetworks,
    NetworksDetected(Vec<LocalNetwork>),
    WindowResized(iced::Size),
    CloseRequested,
    /// Put text on the clipboard; the label names it in the status line.
    CopyText(String, &'static str),
    CopyVisibleRows,
    EscapePressed,
    EnterPressed,
    /// Does nothing; lets a text field claim the Enter key.
    Noop,
    PresetPicked(PresetChoice),
    ShowSavePreset,
    PresetNameChanged(String),
    SavePreset,
    DeletePreset(usize),
    ShowSettings(SettingsTab),
    ShowHostActions(String),
    CloseModal,
    BannersToggled(bool),
    PortToggled(u16, bool),
    SelectAllPorts,
    SelectNoPorts,
    CustomPortsChanged(String),
    ColumnToggled(Column, bool),
    ShowColumnMenu(Column),
    ColumnDragStarted(Column),
    ColumnDraggedOver(Column),
    ColumnDragEnded,
    AutoRescanPicked(AutoInterval),
    NotifyToggled(bool),
    IgnoreVirtualToggled(bool),
    CardViewToggled,
    LabelDraftChanged(String),
    SaveLabel(String),
    RunHostAction(String, HostAction),
    OpenUrl(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Info,
    Error,
}

pub struct LanPing {
    pub target: String,
    pub networks: Vec<LocalNetwork>,
    pub settings: Settings,
    pub labels: DeviceLabels,
    pub modal: Option<Modal>,
    pub label_draft: String,
    pub preset_name_draft: String,
    pub filter: HostFilter,
    pub filters_open: bool,
    /// Appearances of the device shown in the detail dialog, newest scan first.
    pub detail_sightings: Vec<Sighting>,
    /// How many saved scans were searched for those appearances.
    pub detail_scan_count: usize,
    /// Column being dragged in the column list.
    pub column_drag: Option<Column>,
    pub hosts: Vec<Host>,
    pub diff: ScanDiff,
    pub sort_key: SortKey,
    pub sort_ascending: bool,
    pub scanning: bool,
    pub progress: (usize, usize),
    pub progress_stage: ScanStage,
    pub notice: Option<(Notice, String)>,
    pub history: Vec<HistoryEntry>,
    pub(crate) scan_handle: Option<Handle>,
    pub(crate) scanned_target: String,
    /// Hosts of the previous scan of the same network, for comparison.
    pub(crate) baseline: Option<Vec<Host>>,
    /// Results of an automatic rescan, shown once it finishes.
    pub(crate) incoming: Vec<Host>,
    pub(crate) auto_run: bool,
}

/// Maps Ctrl+F, Enter and Esc to messages. Esc is handled even when a text field
/// has consumed it, so it also clears the filters from inside the search box.
fn shortcut_message(
    event: iced::Event,
    status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    use iced::keyboard::{Event, Key, key::Named};
    let iced::Event::Keyboard(Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    match key.as_ref() {
        Key::Named(Named::Escape) => Some(Message::EscapePressed),
        Key::Character("f") if modifiers.command() => Some(Message::FocusSearch),
        Key::Named(Named::Enter) if status == iced::event::Status::Ignored => {
            Some(Message::EnterPressed)
        }
        _ => None,
    }
}

/// The target of the first listed network in `new` when it differs from the first in `old`.
fn changed_primary_target(old: &[LocalNetwork], new: &[LocalNetwork]) -> Option<String> {
    let primary = new.first()?;
    (old.first() != Some(primary)).then(|| primary.target.clone())
}

/// Detected networks the picker offers, honoring the virtual adapter setting.
fn listed_networks(networks: &[LocalNetwork], settings: &Settings) -> Vec<LocalNetwork> {
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
fn matching_networks(networks: Vec<LocalNetwork>, typed: &str) -> Vec<LocalNetwork> {
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

impl LanPing {
    pub fn new() -> (Self, Task<Message>) {
        let migration = migrate_legacy_data();
        let networks = detect_local_networks();
        let (settings, settings_error) = load_settings();
        let target = listed_networks(&networks, &settings).first().map_or_else(
            || FALLBACK_TARGET.to_owned(),
            |network| network.target.clone(),
        );
        let (labels, labels_error) = load_labels();
        let startup_error = settings_error.or(labels_error);
        let startup_notice = match (startup_error, migration) {
            (Some(error), _) => Some((Notice::Error, error)),
            (None, Err(error)) => Some((
                Notice::Error,
                format!("Could not move saved data from the old lansweep folders: {error}"),
            )),
            (None, Ok(0)) => None,
            (None, Ok(_)) => Some((
                Notice::Info,
                "Moved your settings and saved scans from the old lansweep folders".to_owned(),
            )),
        };
        let last_filter = settings.last_filter.clone();
        let mut app = Self {
            target,
            networks,
            settings,
            labels,
            modal: None,
            label_draft: String::new(),
            preset_name_draft: String::new(),
            filter: last_filter,
            filters_open: false,
            detail_sightings: Vec::new(),
            detail_scan_count: 0,
            column_drag: None,
            hosts: Vec::new(),
            diff: ScanDiff::default(),
            sort_key: SortKey::Address,
            sort_ascending: true,
            scanning: false,
            progress: (0, 0),
            progress_stage: ScanStage::Sweep,
            notice: startup_notice,
            history: Vec::new(),
            scan_handle: None,
            scanned_target: String::new(),
            baseline: None,
            incoming: Vec::new(),
            auto_run: false,
        };
        app.refresh_history();
        if let Some(path) = std::env::args_os().nth(1) {
            app.open_scan_file(PathBuf::from(path));
        }
        let first_scan = if app.settings.scan_on_startup {
            Task::done(Message::StartScan)
        } else {
            Task::none()
        };
        (app, first_scan)
    }

    pub fn theme(&self) -> Theme {
        crate::theme::app_theme()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let rescan = match self.settings.auto_rescan_minutes {
            0 => Subscription::none(),
            minutes => iced::time::every(Duration::from_secs(u64::from(minutes) * 60))
                .map(|_| Message::AutoRescanTick),
        };
        let network_watch = if self.settings.scan_on_network_change {
            iced::time::every(NETWORK_CHECK_INTERVAL).map(|_| Message::CheckNetworks)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            rescan,
            network_watch,
            iced::event::listen_with(shortcut_message),
            iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            iced::window::close_requests().map(|_| Message::CloseRequested),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::FocusSearch => return iced::widget::operation::focus(SEARCH_INPUT_ID),
            Message::CopyText(text, label) => {
                self.set_notice(Notice::Info, format!("Copied {label}"));
                return iced::clipboard::write(text);
            }
            Message::CopyVisibleRows => {
                let rows = self.visible_rows();
                let text = rows_as_text(&rows, &self.settings.visible_columns());
                let count = rows.len();
                self.set_notice(Notice::Info, format!("Copied {count} rows"));
                return iced::clipboard::write(text);
            }
            Message::WindowResized(size) => {
                self.settings.window_size = Some((size.width as u32, size.height as u32));
            }
            Message::CloseRequested => {
                self.settings.last_filter = self.filter.clone();
                self.save_settings();
                return iced::exit();
            }
            Message::CheckNetworks => {
                return Task::perform(
                    async {
                        tokio::task::spawn_blocking(detect_local_networks)
                            .await
                            .unwrap_or_default()
                    },
                    Message::NetworksDetected,
                );
            }
            Message::NetworksDetected(detected) => return self.apply_detected_networks(detected),
            Message::DiagnosticFinished(title, result) => self.finish_diagnostic(&title, result),
            Message::ShowHostDetails(key) => self.show_host_details(key),
            Message::EscapePressed => self.escape(),
            Message::EnterPressed => {
                if self.modal.is_none() && !self.scanning {
                    return self.start(false);
                }
            }
            Message::Noop => {}
            Message::TargetChanged(text) => self.target = text,
            Message::NetworkPicked(network) => {
                if !self.scanning {
                    self.target = network.target;
                }
            }
            Message::StartScan => return self.start(false),
            Message::AutoRescanTick => {
                if !self.scanning {
                    return self.start(true);
                }
            }
            Message::StopScan => self.stop("Scan stopped"),
            Message::Scan(event) => self.handle_scan_event(event),
            Message::SortBy(key) => self.toggle_sort(key),
            Message::SortKeyPicked(key) => self.sort_key = key,
            Message::SortDirectionToggled => self.sort_ascending = !self.sort_ascending,
            Message::ExportPicked(format) => return self.ask_export_path(format),
            Message::ExportPathChosen(format, Some(path)) => self.export(format, &path),
            Message::ExportPathChosen(_, None) => {}
            Message::HistoryPicked(entry) => self.load_history(&entry),
            Message::ShowSettings(tab) => self.modal = Some(Modal::Settings(tab)),
            Message::ShowHostActions(key) => self.show_host_actions(key),
            Message::ShowColumnMenu(column) => self.modal = Some(Modal::ColumnMenu(column)),
            Message::ColumnDragStarted(column) => self.column_drag = Some(column),
            Message::ColumnDraggedOver(target) => {
                if let Some(dragged) = self.column_drag {
                    self.settings.move_column_to(dragged, target);
                }
            }
            Message::ColumnDragEnded => {
                if self.column_drag.take().is_some() {
                    self.save_settings();
                }
            }
            Message::CloseModal => self.modal = None,
            Message::LabelDraftChanged(text) => self.label_draft = text,
            Message::SaveLabel(key) => self.save_label(&key),
            Message::RunHostAction(key, action) => return self.run_host_action(&key, &action),
            Message::OpenUrl(url) => self.open_url(&url),
            other => self.apply_filter_or_settings_change(other),
        }
        Task::none()
    }

    fn apply_filter_or_settings_change(&mut self, message: Message) {
        match message {
            Message::Filter(change) => self.filter.apply(change),
            Message::FiltersPanelToggled => self.filters_open = !self.filters_open,
            Message::PresetPicked(choice) => self.filter = choice.preset.filter,
            Message::ShowSavePreset => {
                self.preset_name_draft.clear();
                self.modal = Some(Modal::SavePreset);
            }
            Message::PresetNameChanged(text) => self.preset_name_draft = text,
            Message::SavePreset => self.save_preset(),
            settings_change => self.apply_settings_change(settings_change),
        }
    }

    fn apply_settings_change(&mut self, message: Message) {
        match message {
            Message::BannersToggled(enabled) => self.settings.grab_banners = enabled,
            Message::PortToggled(port, selected) => self.settings.set_port_selected(port, selected),
            Message::SelectAllPorts => self.settings.select_all_ports(),
            Message::SelectNoPorts => self.settings.select_no_ports(),
            Message::CustomPortsChanged(text) => self.settings.custom_ports = text,
            Message::ColumnToggled(column, visible) => {
                self.settings.set_column_visible(column, visible);
            }
            Message::AutoRescanPicked(interval) => self.settings.auto_rescan_minutes = interval.0,
            Message::NotifyToggled(enabled) => self.settings.notify_new_devices = enabled,
            Message::CardViewToggled => self.settings.card_view = !self.settings.card_view,
            Message::ScanOnStartupToggled(enabled) => self.settings.scan_on_startup = enabled,
            Message::ScanOnNetworkChangeToggled(enabled) => {
                self.settings.scan_on_network_change = enabled;
            }
            Message::IgnoreVirtualToggled(ignore) => self.settings.ignore_virtual_networks = ignore,
            Message::DeletePreset(index) => {
                if index < self.settings.filter_presets.len() {
                    self.settings.filter_presets.remove(index);
                }
            }
            _ => return,
        }
        self.save_settings();
    }

    fn save_preset(&mut self) {
        let result = upsert_preset(
            &mut self.settings.filter_presets,
            &self.preset_name_draft,
            self.filter.clone(),
        );
        match result {
            Ok(()) => {
                self.modal = None;
                self.save_settings();
                self.set_notice(
                    Notice::Info,
                    format!("Saved preset {}", self.preset_name_draft.trim()),
                );
            }
            Err(error) => self.set_notice(Notice::Error, error),
        }
    }

    pub(crate) fn save_settings(&mut self) {
        let Some(path) = settings_path() else { return };
        if let Err(error) = self.settings.save(&path) {
            self.set_notice(Notice::Error, format!("Could not save settings: {error}"));
        }
    }

    fn show_host_actions(&mut self, key: String) {
        self.label_draft = self.labels.get(&key).unwrap_or_default().to_owned();
        self.modal = Some(Modal::HostActions(key));
    }

    fn save_label(&mut self, key: &str) {
        self.labels.set(key, &self.label_draft);
        let Some(path) = labels_path() else { return };
        match self.labels.save(&path) {
            Ok(()) => self.set_notice(Notice::Info, "Device name saved".to_owned()),
            Err(error) => self.set_notice(
                Notice::Error,
                format!("Could not save device name: {error}"),
            ),
        }
    }

    fn toggle_sort(&mut self, key: SortKey) {
        if self.sort_key == key {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_key = key;
            self.sort_ascending = true;
        }
    }

    /// Remembers the detected networks. When the main one changed and the
    /// setting is on, points the target at it and scans.
    fn apply_detected_networks(&mut self, detected: Vec<LocalNetwork>) -> Task<Message> {
        let new_target = changed_primary_target(
            &listed_networks(&self.networks, &self.settings),
            &listed_networks(&detected, &self.settings),
        );
        self.networks = detected;
        match new_target {
            Some(target) if self.settings.scan_on_network_change && !self.scanning => {
                self.target = target;
                self.start(false)
            }
            _ => Task::none(),
        }
    }

    /// Esc closes the open dialog, or else clears the filters.
    fn escape(&mut self) {
        if self.modal.is_some() {
            self.modal = None;
        } else if self.filter.active_count() > 0 {
            self.filter = HostFilter::default();
        }
    }

    /// Networks to suggest under the target field for what is typed so far.
    pub fn network_suggestions(&self) -> Vec<LocalNetwork> {
        if self.scanning {
            return Vec::new();
        }
        matching_networks(
            listed_networks(&self.networks, &self.settings),
            &self.target,
        )
    }

    /// A device from the live list or, failing that, the gone list.
    pub fn find_host(&self, key: &str) -> Option<&Host> {
        self.hosts
            .iter()
            .chain(self.diff.gone.iter())
            .find(|host| host.device_key() == key)
    }

    /// Live hosts followed by gone ones, filtered and ordered for display.
    pub fn visible_rows(&self) -> Vec<Row<'_>> {
        let live = self
            .hosts
            .iter()
            .map(|host| (host, self.diff.status(host.ip).into()));
        let gone = self.diff.gone.iter().map(|host| (host, RowStatus::Gone));
        let mut rows: Vec<Row<'_>> = live
            .chain(gone)
            .map(|(host, status)| Row {
                host,
                status,
                label: self.labels.get(&host.device_key()),
            })
            .filter(|row| self.filter.matches(row.host, row.label, row.status))
            .collect();
        rows.sort_by(|a, b| {
            let gone_last = (a.status == RowStatus::Gone).cmp(&(b.status == RowStatus::Gone));
            let order = compare_hosts(a.host, b.host, self.sort_key);
            let order = if self.sort_ascending {
                order
            } else {
                order.reverse()
            };
            gone_last.then(order)
        });
        rows
    }

    fn open_url(&mut self, url: &str) {
        if let Err(error) = open_target(url) {
            self.set_notice(Notice::Error, format!("Could not open {url}: {error}"));
        }
    }

    fn run_host_action(&mut self, key: &str, action: &HostAction) -> Task<Message> {
        self.modal = None;
        let Some(effect) = self.find_host(key).and_then(|host| action.effect(host)) else {
            self.set_notice(
                Notice::Error,
                "That detail is not known for this host".to_owned(),
            );
            return Task::none();
        };
        match effect {
            Effect::Open(target) => {
                match open_target(&target) {
                    Ok(()) => self.set_notice(Notice::Info, format!("Opened {target}")),
                    Err(error) => {
                        self.set_notice(Notice::Error, format!("Could not open {target}: {error}"));
                    }
                }
                Task::none()
            }
            Effect::Copy(text) => {
                self.set_notice(Notice::Info, format!("Copied {text}"));
                iced::clipboard::write(text)
            }
            Effect::Diagnose(diagnostic, ip) => {
                let title = format!("{diagnostic} {ip}");
                self.modal = Some(Modal::Diagnostic {
                    title: title.clone(),
                    output: None,
                });
                Task::perform(run_diagnostic(diagnostic, ip), move |result| {
                    Message::DiagnosticFinished(title.clone(), result)
                })
            }
            Effect::Wake(mac) => {
                match send_wake_packet(&mac) {
                    Ok(()) => {
                        self.set_notice(Notice::Info, format!("Sent wake-up packet to {mac}"))
                    }
                    Err(error) => self.set_notice(
                        Notice::Error,
                        format!("Could not send wake-up packet: {error}"),
                    ),
                }
                Task::none()
            }
        }
    }

    fn show_host_details(&mut self, key: String) {
        let scans: Vec<SavedScan> = history_directory()
            .map(|directory| {
                list_scans(&directory)
                    .into_iter()
                    .map(|(_, scan)| scan)
                    .collect()
            })
            .unwrap_or_default();
        self.detail_scan_count = scans.len();
        self.detail_sightings = sightings(&scans, &key);
        self.modal = Some(Modal::HostDetails(key));
    }

    /// How a listed device compares with the previous scan.
    pub fn row_status(&self, host: &Host) -> RowStatus {
        if self.diff.gone.iter().any(|gone| gone.ip == host.ip) {
            RowStatus::Gone
        } else {
            self.diff.status(host.ip).into()
        }
    }

    /// Shows the output in the dialog that is waiting for it, if it is still open.
    fn finish_diagnostic(&mut self, finished_title: &str, result: Result<String, String>) {
        if let Some(Modal::Diagnostic { title, output }) = &mut self.modal
            && title == finished_title
        {
            *output = Some(result.unwrap_or_else(|error| error));
        }
    }

    fn ask_export_path(&mut self, format: ExportFormat) -> Task<Message> {
        if self.hosts.is_empty() {
            self.set_notice(Notice::Error, "Nothing to export yet".to_owned());
            return Task::none();
        }
        let file_name = format!("lanping-{}.{}", unix_now(), format.extension());
        Task::perform(
            ask_save_path(format, file_name, export_directory()),
            move |path| Message::ExportPathChosen(format, path),
        )
    }

    fn export(&mut self, format: ExportFormat, path: &Path) {
        let result = format
            .render(&self.hosts)
            .and_then(|content| fs::write(path, content).map_err(|error| error.to_string()));
        match result {
            Ok(()) => self.set_notice(Notice::Info, format!("Saved {}", path.display())),
            Err(error) => self.set_notice(Notice::Error, format!("Could not save export: {error}")),
        }
    }

    pub(crate) fn save_to_history(&mut self) {
        let Some(directory) = history_directory() else {
            return;
        };
        let scan = SavedScan::new(self.scanned_target.clone(), self.hosts.clone());
        if let Err(error) = save_scan(&directory, &scan) {
            self.set_notice(
                Notice::Error,
                format!("Could not save scan history: {error}"),
            );
            return;
        }
        self.refresh_history();
    }

    pub(crate) fn refresh_history(&mut self) {
        let Some(directory) = history_directory() else {
            return;
        };
        self.history = list_scans(&directory)
            .into_iter()
            .map(|(path, scan)| HistoryEntry {
                path,
                label: format!(
                    "{} - {} ({} hosts)",
                    format_unix_utc(scan.finished_at_unix),
                    scan.target,
                    scan.hosts.len()
                ),
                target: scan.target,
            })
            .collect();
    }

    /// Hosts of the newest saved scan of `target`.
    pub(crate) fn previous_scan_hosts(&self, target: &str) -> Option<Vec<Host>> {
        let entry = self.history.iter().find(|entry| entry.target == target)?;
        load_scan(&entry.path).ok().map(|scan| scan.hosts)
    }

    /// Opens a saved scan named on the command line, for example a demo or one from another machine.
    fn open_scan_file(&mut self, path: PathBuf) {
        let entry = self
            .history
            .iter()
            .find(|entry| entry.path == path)
            .cloned()
            .unwrap_or_else(|| HistoryEntry {
                label: path.display().to_string(),
                target: String::new(),
                path,
            });
        self.load_history(&entry);
    }

    fn load_history(&mut self, entry: &HistoryEntry) {
        self.stop("Scan stopped");
        match load_scan(&entry.path) {
            Ok(scan) => {
                let earlier = self
                    .history
                    .iter()
                    .skip_while(|candidate| candidate.path != entry.path)
                    .skip(1)
                    .find(|candidate| candidate.target == scan.target)
                    .and_then(|candidate| load_scan(&candidate.path).ok());
                self.diff = match earlier {
                    Some(earlier) => diff_scans(&earlier.hosts, &scan.hosts),
                    None => ScanDiff::default(),
                };
                self.target = scan.target;
                self.hosts = scan.hosts;
                self.progress = (0, 0);
                self.set_notice(Notice::Info, format!("Loaded {}", entry.label));
            }
            Err(error) => {
                self.set_notice(Notice::Error, format!("Could not open saved scan: {error}"));
            }
        }
    }

    pub(crate) fn set_notice(&mut self, kind: Notice, text: String) {
        self.notice = Some((kind, text));
    }
}

/// Reads saved preferences. A missing file means first run; an unreadable
/// one is reported so the user knows their choices were not applied.
pub(crate) fn load_settings() -> (Settings, Option<String>) {
    let Some(path) = settings_path() else {
        return (Settings::default(), None);
    };
    match Settings::load(&path) {
        Ok(settings) => (settings, None),
        Err(error) if error.kind() == ErrorKind::NotFound => (Settings::default(), None),
        Err(error) => (
            Settings::default(),
            Some(format!("Could not read settings, using defaults: {error}")),
        ),
    }
}

fn load_labels() -> (DeviceLabels, Option<String>) {
    let Some(path) = labels_path() else {
        return (DeviceLabels::default(), None);
    };
    match DeviceLabels::load(&path) {
        Ok(labels) => (labels, None),
        Err(error) if error.kind() == ErrorKind::NotFound => (DeviceLabels::default(), None),
        Err(error) => (
            DeviceLabels::default(),
            Some(format!("Could not read device names: {error}")),
        ),
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
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

    fn press(key: iced::keyboard::Key, modifiers: iced::keyboard::Modifiers) -> iced::Event {
        use iced::keyboard::{Event, Location, key::Physical};
        iced::Event::Keyboard(Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: Physical::Unidentified(iced::keyboard::key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        })
    }

    #[test]
    fn shortcuts_map_to_messages() {
        use iced::event::Status::{Captured, Ignored};
        use iced::keyboard::{Key, Modifiers, key::Named};
        let id = iced::window::Id::unique();
        let map = |key, modifiers, status| shortcut_message(press(key, modifiers), status, id);
        assert!(matches!(
            map(Key::Named(Named::Escape), Modifiers::empty(), Captured),
            Some(Message::EscapePressed)
        ));
        assert!(matches!(
            map(Key::Character("f".into()), Modifiers::CTRL, Ignored),
            Some(Message::FocusSearch)
        ));
        assert!(map(Key::Character("f".into()), Modifiers::empty(), Ignored).is_none());
        assert!(matches!(
            map(Key::Named(Named::Enter), Modifiers::empty(), Ignored),
            Some(Message::EnterPressed)
        ));
        assert!(map(Key::Named(Named::Enter), Modifiers::empty(), Captured).is_none());
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
