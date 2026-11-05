use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use iced::task::Handle;
use iced::{Subscription, Task, Theme};
use lanping_core::device_notes::{DeviceNote, DeviceNotes};
use lanping_core::diagnostics::run as run_diagnostic;
use lanping_core::diff::{ScanDiff, diff_scans};
use lanping_core::dns::{Answer, DnsError, RecordType, ResolverTiming};
use lanping_core::gateway::default_gateway;
use lanping_core::history::{
    SavedScan, Sighting, list_scan_summaries, list_scans, load_scan, save_scan, sightings,
};
use lanping_core::labels::DeviceLabels;
use lanping_core::latency::{LatencyError, Prober};
use lanping_core::measurements::{Measurement, save as save_measurement};
use lanping_core::ndt7::Server;
use lanping_core::ntp::{NtpError, NtpReport};
use lanping_core::range::{LocalNetwork, detect_local_networks};
use lanping_core::report::ReportInput;
use lanping_core::snmp_interfaces::{Interface, Poll, SnmpError};
use lanping_core::summary::{Summary, summarize};
use lanping_core::throughput::ServedStream;
use lanping_core::timeline::{TimelineEntry, device_timeline};
use lanping_core::tls::{TlsError, TlsReport};
use lanping_core::trace::{HopSample, TraceError};
use lanping_core::whois::{WhoisError, WhoisReport};
use lanping_core::wifi::{AccessPoint, WifiError};
use lanping_core::wol::send_wake_packet;
use lanping_core::{Host, ScanEvent, ScanStage};

use crate::actions::{Effect, HostAction};
use crate::copy_text::rows_as_text;
use crate::dns_tab::{DnsPage, DnsState, ResolverChoice};
use crate::export_format::{ExportFormat, ask_save_path};
use crate::host_filter::{FilterChange, HostFilter, RowStatus};
use crate::host_order::SortKey;
use crate::host_rows::Row;
use crate::latency_tab::{Interval, LatencyState};
use crate::monitor::AutoInterval;
use crate::ntp_tab::NtpState;
use crate::opener::open_target;
use crate::presets::{PresetChoice, upsert_preset};
use crate::row_list::{RowInputs, RowList};
use crate::settings::{Column, Settings, SettingsTab};
use crate::snmp_tab::SnmpState;
use crate::speed_locations::LocationChoice;
use crate::speed_model::{Source, SpeedEvent, SpeedPage, SpeedService, TestKind, TestLength};
use crate::speed_tab::SpeedState;
use crate::storage::{
    device_notes_path, export_directory, history_directory, labels_path, load_or_default,
    measurements_directory, migrate_legacy_data, settings_path,
};
use crate::subnet_tab::SubnetState;
use crate::tabs::Tab;
use crate::timestamp::{format_unix_utc, unix_now};
use crate::tls_tab::TlsState;
use crate::trace_tab::TraceState;
use crate::wifi_list::{WifiFilterChange, WifiSortKey};
use crate::wifi_tab::WifiState;

mod files;
mod host_actions;
mod message;
mod network_list;
mod shortcuts;

pub use message::{HistoryEntry, Message, Modal, NoteDraft, NoteField, Notice};

/// How often the interface list is read while "scan when the network changes" is on.
const NETWORK_CHECK_INTERVAL: Duration = Duration::from_secs(10);
/// How often the Wi-Fi tab scans while "Auto refresh" is on.
const WIFI_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
const FALLBACK_TARGET: &str = "192.168.0.0/24";
/// Widget id of the results search box, so Ctrl+F can focus it.
pub const SEARCH_INPUT_ID: &str = "search";

pub struct LanPing {
    pub target: String,
    pub networks: Vec<LocalNetwork>,
    pub settings: Settings,
    pub labels: DeviceLabels,
    pub tab: Tab,
    pub settings_tab: SettingsTab,
    pub wifi: WifiState,
    pub trace: TraceState,
    pub subnet: SubnetState,
    pub latency: LatencyState,
    pub dns: DnsState,
    pub tls: TlsState,
    pub ntp: NtpState,
    pub snmp: SnmpState,
    pub speed: SpeedState,
    pub modal: Option<Modal>,
    pub label_draft: String,
    pub notes: DeviceNotes,
    pub note_draft: NoteDraft,
    pub preset_name_draft: String,
    pub filter: HostFilter,
    pub filters_open: bool,
    /// Appearances of the device shown in the detail dialog, newest scan first.
    pub detail_sightings: Vec<Sighting>,
    /// How many saved scans were searched for those appearances.
    pub detail_scan_count: usize,
    /// What changed about that device over time, newest first.
    pub detail_timeline: Vec<TimelineEntry>,
    /// Column being dragged in the column list.
    pub column_drag: Option<Column>,
    /// Tab being dragged in the tab list of the settings.
    pub tab_drag: Option<Tab>,
    pub hosts: Vec<Host>,
    pub row_list: RowList,
    /// Every saved scan, read when first needed and dropped when a scan is saved.
    saved_scans: Option<Vec<SavedScan>>,
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

use network_list::{changed_primary_target, listed_networks, matching_networks};
use shortcuts::shortcut_message;

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
        let (notes, notes_error) = load_notes();
        let startup_error = settings_error.or(labels_error).or(notes_error);
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
            tab: Tab::default(),
            settings_tab: SettingsTab::default(),
            wifi: WifiState::default(),
            trace: TraceState::default(),
            subnet: SubnetState::default(),
            latency: LatencyState::default(),
            dns: DnsState::new(),
            tls: TlsState::new(),
            ntp: NtpState::new(),
            snmp: SnmpState::new(),
            speed: SpeedState::new(measurements_directory().as_deref()),
            modal: None,
            label_draft: String::new(),
            notes,
            note_draft: NoteDraft::default(),
            preset_name_draft: String::new(),
            filter: last_filter,
            filters_open: false,
            detail_sightings: Vec::new(),
            detail_scan_count: 0,
            detail_timeline: Vec::new(),
            column_drag: None,
            tab_drag: None,
            hosts: Vec::new(),
            row_list: RowList::default(),
            saved_scans: None,
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
        app.refresh_rows();
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
        let wifi_refresh = if self.tab == Tab::Wifi && self.wifi.auto_refresh() {
            iced::time::every(WIFI_REFRESH_INTERVAL).map(|_| Message::WifiRefresh)
        } else {
            Subscription::none()
        };
        Subscription::batch([
            rescan,
            network_watch,
            wifi_refresh,
            iced::event::listen_with(shortcut_message),
            iced::window::resize_events().map(|(_, size)| Message::WindowResized(size)),
            iced::window::close_requests().map(|_| Message::CloseRequested),
        ])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);
        // The rows only show on the devices tab. Going back to it is a message too, so they are
        // up to date by the time they are drawn.
        if self.tab == Tab::Scan {
            self.refresh_rows();
        }
        self.close_dialog_of_a_gone_host();
        task
    }

    /// A menu or details dialog for a device that has left the list (a new scan, or a saved scan
    /// opened) shows nothing, yet would still block Enter and Esc. It is closed instead.
    fn close_dialog_of_a_gone_host(&mut self) {
        if let Some(Modal::HostActions(key) | Modal::HostDetails(key)) = &self.modal
            && self.find_host(key).is_none()
        {
            self.modal = None;
        }
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
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
            Message::ShowSummary => self.show_summary(),
            Message::ShowNetworkMap => {
                self.modal = Some(Modal::NetworkMap(default_gateway()));
            }
            Message::EscapePressed => self.escape(),
            Message::EnterPressed => {
                if self.tab == Tab::Scan && self.modal.is_none() && !self.scanning {
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
            Message::ShowSettings(tab) => {
                self.modal = None;
                self.settings_tab = tab;
                self.tab = Tab::Settings;
            }
            Message::ShowHostActions(key) => self.show_host_actions(key),
            Message::ShowColumnMenu(column) => self.modal = Some(Modal::ColumnMenu(column)),
            Message::ColumnDragStarted(column) => self.column_drag = Some(column),
            Message::ColumnDraggedOver(target) => {
                if let Some(dragged) = self.column_drag {
                    self.settings.move_column_to(dragged, target);
                }
            }
            Message::TabToggled(tab, visible) => {
                self.settings.set_tab_visible(tab, visible);
                if !visible && self.tab == tab {
                    self.tab = Tab::Scan;
                }
                self.save_settings();
            }
            Message::TabDragStarted(tab) => self.tab_drag = Some(tab),
            Message::TabDraggedOver(target) => {
                if let Some(dragged) = self.tab_drag {
                    self.settings.move_tab_to(dragged, target);
                }
            }
            Message::TabDragEnded => {
                if self.tab_drag.take().is_some() {
                    self.save_settings();
                }
            }
            Message::ColumnDragEnded => {
                if self.column_drag.take().is_some() {
                    self.save_settings();
                }
            }
            Message::CloseModal => self.modal = None,
            Message::LabelDraftChanged(text) => self.label_draft = text,
            Message::NoteDraftChanged(field, text) => self.note_draft.set(field, text),
            Message::SaveNote(key) => self.save_note(&key),
            Message::SaveLabel(key) => self.save_label(&key),
            Message::RunHostAction(key, action) => return self.run_host_action(&key, &action),
            Message::OpenUrl(url) => self.open_url(&url),
            Message::TabSelected(tab) => {
                self.tab = tab;
                if tab == Tab::Wifi && !self.wifi.has_scanned() {
                    return self.wifi.start();
                }
            }
            Message::WifiRefresh => return self.wifi.start(),
            Message::WifiAutoRefreshToggled(on) => self.wifi.set_auto_refresh(on),
            Message::WifiSortBy(key) => self.wifi.sort_by(key),
            Message::WifiFilter(change) => self.wifi.apply_filter(change),
            Message::LatencyTargetChanged(target) => self.latency.set_target(target),
            Message::LatencyIntervalPicked(interval) => self.latency.set_interval(interval),
            Message::LatencyStart => return self.latency.start(),
            Message::LatencyStop => {
                let finished_run = self.latency.record_for_history();
                self.latency.stop();
                if let Some(record) = finished_run {
                    self.save_measurements(&[record]);
                }
            }
            Message::LatencyReady(generation, result) => {
                return self.latency.ready(generation, result);
            }
            Message::LatencySample(generation, rtt_ms) => {
                return self.latency.sampled(generation, rtt_ms);
            }
            Message::LatencyCopySummary => {
                self.set_notice(Notice::Info, "Copied latency summary".to_owned());
                return iced::clipboard::write(self.latency.summary());
            }
            Message::LatencySaveLog => {
                return Task::perform(
                    ask_save_path(
                        ExportFormat::Csv,
                        self.latency.default_log_name(),
                        export_directory(),
                    ),
                    Message::LatencyLogPathChosen,
                );
            }
            Message::LatencyLogPathChosen(Some(path)) => self.save_latency_log(&path),
            Message::LatencyLogPathChosen(None) => {}
            Message::SpeedPagePicked(page) => self.speed.page = page,
            Message::SpeedServicePicked(service) => return self.speed.set_service(service),
            Message::SpeedLocationPicked(location) => return self.speed.set_location(location),
            Message::SpeedCountriesFound(result) => self.speed.countries_found(result),
            Message::SpeedServersFound(generation, result) => {
                self.speed.servers_found(generation, result);
            }
            Message::SpeedServerPicked(server) => self.speed.set_server(server),
            Message::SpeedServersSearchAgain => return self.speed.search_again(),
            Message::SpeedLanTargetChanged(target) => self.speed.lan_target = target,
            Message::SpeedLengthPicked(length) => self.speed.length = length,
            Message::SpeedKindPicked(kind) => self.speed.kind = kind,
            Message::SpeedStart(source) => return self.speed.start_test(source),
            Message::SpeedStop => {
                let finished = self.speed.stop_test();
                self.save_measurements(&finished);
            }
            Message::SpeedEvent(generation, event) => {
                let finished = self.speed.event(generation, event);
                self.save_measurements(&finished);
            }
            Message::SpeedServerPortChanged(port) => self.speed.server_port = port,
            Message::SpeedServerStart => return self.speed.start_server(),
            Message::SpeedServerStop => self.speed.stop_server(),
            Message::SpeedServerListening(port, addresses) => {
                self.speed.server_listening(port, addresses);
            }
            Message::SpeedServerFailed(message) => self.speed.server_failed(message),
            Message::SpeedServed(stream) => self.speed.served(stream),
            Message::SnmpTargetChanged(target) => self.snmp.target = target,
            Message::SnmpStart => {
                return self.snmp.start(&self.settings.snmp_community);
            }
            Message::SnmpStop => self.snmp.stop(),
            Message::SnmpDiscovered(generation, result) => {
                return self.snmp.discovered(generation, result);
            }
            Message::SnmpPolled(generation, result) => {
                return self.snmp.polled(generation, result);
            }
            Message::SnmpShowAllToggled(show_all) => self.snmp.show_all = show_all,
            Message::TlsTargetChanged(target) => self.tls.set_target(target),
            Message::TlsServerNameChanged(name) => self.tls.set_server_name(name),
            Message::TlsCheck => return self.tls.start(),
            Message::TlsDone(generation, result) => self.tls.finish(generation, result),
            Message::NtpServerChanged(server) => self.ntp.set_server(server),
            Message::NtpCheck => return self.ntp.start(),
            Message::NtpDone(generation, result) => self.ntp.finish(generation, result),
            Message::DnsPagePicked(page) => self.dns.set_page(page),
            Message::WhoisQueryChanged(query) => self.dns.whois.set_query(query),
            Message::WhoisLookup => return self.dns.whois.start(),
            Message::WhoisDone(generation, result) => self.dns.whois.finish(generation, result),
            Message::WhoisCopy => {
                if let Some(answer) = self.dns.whois.answer_text() {
                    let answer = answer.to_owned();
                    self.set_notice(Notice::Info, "Copied the WHOIS answer".to_owned());
                    return iced::clipboard::write(answer);
                }
            }
            Message::DnsNameChanged(name) => self.dns.set_name(name),
            Message::DnsTypePicked(record_type) => self.dns.set_record_type(record_type),
            Message::DnsResolverPicked(resolver) => self.dns.set_resolver(resolver),
            Message::DnsLookup => return self.dns.start_lookup(),
            Message::DnsLookupDone(generation, result) => {
                self.dns.finish_lookup(generation, result);
            }
            Message::DnsCompare => return self.dns.start_comparison(),
            Message::DnsComparisonDone(generation, timings) => {
                self.dns.finish_comparison(generation, timings);
            }
            Message::DnsCopyRecords => {
                let (text, count) = self.dns.records_as_text();
                self.set_notice(Notice::Info, format!("Copied {count} records"));
                return iced::clipboard::write(text);
            }
            Message::SubnetInputChanged(input) => self.subnet.set_input(input),
            Message::SubnetScanRange(target) => {
                self.target = target;
                self.tab = Tab::Scan;
            }
            Message::TraceTargetChanged(target) => self.trace.set_target(target),
            Message::TraceStart => return self.trace.start(),
            Message::TraceStop => self.trace.stop(),
            Message::TraceRound(generation, result) => {
                return self.trace.finish_round(generation, result);
            }
            Message::TraceCopyRows => {
                let (text, count) = self.trace.stats_as_text();
                self.set_notice(Notice::Info, format!("Copied {count} hops"));
                return iced::clipboard::write(text);
            }
            Message::WifiCopyRows => {
                let (text, count) = self.wifi.rows_as_text();
                self.set_notice(Notice::Info, format!("Copied {count} networks"));
                return iced::clipboard::write(text);
            }
            Message::WifiScanned(result) => self.wifi.finish(result),
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
            Message::SnmpCommunityChanged(text) => self.settings.snmp_community = text,
            Message::PortToggled(port, selected) => self.settings.set_port_selected(port, selected),
            Message::SelectAllPorts => self.settings.select_all_ports(),
            Message::SelectNoPorts => self.settings.select_no_ports(),
            Message::CustomPortsChanged(text) => self.settings.custom_ports = text,
            Message::ColumnToggled(column, visible) => {
                self.settings.set_column_visible(column, visible);
            }
            Message::AutoRescanPicked(interval) => self.settings.auto_rescan_minutes = interval.0,
            Message::NotifyToggled(enabled) => self.settings.notify_new_devices = enabled,
            Message::NotifyPortsToggled(enabled) => self.settings.notify_ports_opened = enabled,
            Message::NotifyOfflineToggled(enabled) => self.settings.notify_named_offline = enabled,
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

    fn save_note(&mut self, key: &str) {
        let draft = &self.note_draft;
        let note = DeviceNote::from_input(&draft.owner, &draft.location, &draft.tags, &draft.notes);
        self.notes.set(key, note);
        let Some(path) = device_notes_path() else {
            return;
        };
        match self.notes.save(&path) {
            Ok(()) => self.set_notice(Notice::Info, "Device notes saved".to_owned()),
            Err(error) => {
                self.set_notice(
                    Notice::Error,
                    format!("Could not save device notes: {error}"),
                );
            }
        }
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
        } else if self.tab == Tab::Wifi {
            self.wifi.clear_filter();
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

    /// Live hosts followed by gone ones, filtered and ordered for display. The list is worked
    /// out after each message, so drawing it costs no sorting or filtering.
    pub fn visible_rows(&self) -> Vec<Row<'_>> {
        self.row_list.rows(&self.hosts, &self.diff.gone)
    }

    /// Works the row list out again from the hosts, filter, order, names and notes.
    fn refresh_rows(&mut self) {
        self.row_list.rebuild(&RowInputs {
            hosts: &self.hosts,
            diff: &self.diff,
            labels: &self.labels,
            notes: &self.notes,
            filter: &self.filter,
            sort_key: self.sort_key,
            ascending: self.sort_ascending,
        });
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

    pub(crate) fn set_notice(&mut self, kind: Notice, text: String) {
        self.notice = Some((kind, text));
    }
}

/// Reads saved preferences. A missing file means first run; an unreadable
/// one is reported so the user knows their choices were not applied.
pub(crate) fn load_settings() -> (Settings, Option<String>) {
    load_or_default(settings_path(), "your settings", Settings::load)
}

fn load_saved_scans() -> Vec<SavedScan> {
    history_directory()
        .map(|directory| {
            list_scans(&directory)
                .into_iter()
                .map(|(_, scan)| scan)
                .collect()
        })
        .unwrap_or_default()
}

fn load_notes() -> (DeviceNotes, Option<String>) {
    load_or_default(device_notes_path(), "your device notes", DeviceNotes::load)
}

fn load_labels() -> (DeviceLabels, Option<String>) {
    load_or_default(labels_path(), "your device names", DeviceLabels::load)
}
