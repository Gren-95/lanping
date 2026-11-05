//! Starting, stopping and finishing scans.

use iced::Task;
use iced::futures::SinkExt;
use lanping_core::diff::{ScanDiff, diff_scans};
use lanping_core::range::parse_targets;
use lanping_core::snmp::SnmpCommunity;
use lanping_core::{Host, ScanConfig, ScanEvent, ScanStage, start_scan};

use crate::app::{LanPing, Message, Notice};
use crate::monitor::{AlertRules, scan_alert};
use crate::notify;

const STREAM_BUFFER: usize = 64;

impl LanPing {
    /// Starts a scan. `automatic` scans keep the old list on screen until
    /// they finish and may raise a new-device alert.
    pub(crate) fn start(&mut self, automatic: bool) -> Task<Message> {
        // Never two scans feeding one list.
        if let Some(handle) = self.scan_handle.take() {
            handle.abort();
        }
        let targets = match parse_targets(&self.target) {
            Ok(targets) => targets,
            Err(error) => {
                self.set_notice(Notice::Error, error.to_string());
                return Task::none();
            }
        };
        let ports = match self.ports_to_scan() {
            Ok(ports) => ports,
            Err(message) => {
                self.set_notice(Notice::Error, message);
                return Task::none();
            }
        };

        let scanned_target = self.target.trim().to_owned();
        self.baseline = self.previous_scan_hosts(&scanned_target);
        self.scanned_target = scanned_target;
        self.auto_run = automatic;
        self.incoming.clear();
        if !automatic {
            self.hosts.clear();
            self.diff = ScanDiff::default();
            self.notice = None;
        }
        self.progress = (0, targets.len());
        self.progress_stage = ScanStage::Sweep;
        self.scanning = true;

        let mut config = ScanConfig::new(targets);
        config.grab_banners = self.settings.grab_banners && !ports.is_empty();
        let community = self.settings.snmp_community.trim();
        config.snmp_community =
            (!community.is_empty()).then(|| SnmpCommunity(community.to_owned()));
        config.ports = ports;
        let events = iced::stream::channel(STREAM_BUFFER, async move |mut output| {
            let mut scan = start_scan(config);
            while let Some(event) = scan.next_event().await {
                if output.send(Message::Scan(event)).await.is_err() {
                    break;
                }
            }
        });
        let (task, handle) = Task::stream(events).abortable();
        self.scan_handle = Some(handle);
        task
    }

    /// Ports for the next scan: the ticked and custom ports. Empty means no port scan.
    fn ports_to_scan(&self) -> Result<Vec<u16>, String> {
        self.settings
            .resolve_ports()
            .map_err(|error| error.to_string())
    }

    pub(crate) fn stop(&mut self, reason: &str) {
        if let Some(handle) = self.scan_handle.take() {
            handle.abort();
        }
        if self.scanning {
            self.scanning = false;
            // An automatic scan keeps the old list on screen, so what it found is in `incoming`.
            let found = if self.auto_run {
                self.incoming.len()
            } else {
                self.hosts.len()
            };
            self.incoming.clear();
            self.set_notice(Notice::Info, format!("{reason}: {found} hosts found"));
        }
    }

    pub(crate) fn handle_scan_event(&mut self, event: ScanEvent) {
        if !self.scanning {
            return;
        }
        match event {
            ScanEvent::Progress {
                stage,
                scanned,
                total,
            } => {
                self.progress_stage = stage;
                self.progress = (scanned, total);
            }
            ScanEvent::Host(host) => self.upsert_host(*host),
            ScanEvent::Finished => self.finish_scan(),
        }
    }

    fn upsert_host(&mut self, host: Host) {
        let list = if self.auto_run {
            &mut self.incoming
        } else {
            &mut self.hosts
        };
        match list.iter_mut().find(|known| known.ip == host.ip) {
            Some(known) => *known = host,
            None => list.push(host),
        }
    }

    fn finish_scan(&mut self) {
        self.scan_handle = None;
        self.scanning = false;
        if self.auto_run {
            self.hosts = std::mem::take(&mut self.incoming);
        }
        self.diff = match &self.baseline {
            Some(previous) => diff_scans(previous, &self.hosts),
            None => ScanDiff::default(),
        };
        self.save_to_history();

        let gone = self.diff.gone.len();
        let summary = if gone > 0 {
            format!(
                "Scan finished: {} hosts found, {gone} gone",
                self.hosts.len()
            )
        } else {
            format!("Scan finished: {} hosts found", self.hosts.len())
        };
        self.set_notice(Notice::Info, summary);
        self.alert_about_changes();
    }

    fn alert_about_changes(&mut self) {
        let Some(previous) = self.baseline.as_deref().filter(|_| self.auto_run) else {
            return;
        };
        let rules = AlertRules {
            new_devices: self.settings.notify_new_devices,
            ports_opened: self.settings.notify_ports_opened,
            named_offline: self.settings.notify_named_offline,
        };
        let Some(text) = scan_alert(previous, &self.hosts, &self.diff, &self.labels, rules) else {
            return;
        };
        if let Err(error) = notify::send("LanPing", &text) {
            self.set_notice(
                Notice::Error,
                format!("Could not show notification: {error}"),
            );
        }
    }
}
