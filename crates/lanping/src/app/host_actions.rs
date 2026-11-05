use super::*;

impl LanPing {
    pub(super) fn open_url(&mut self, url: &str) {
        if let Err(error) = open_target(url) {
            self.set_notice(Notice::Error, format!("Could not open {url}: {error}"));
        }
    }

    pub(super) fn run_host_action(&mut self, key: &str, action: &HostAction) -> Task<Message> {
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
            Effect::CheckCertificate {
                target,
                server_name,
            } => {
                // The tab may be hidden in the settings. Showing it keeps the page reachable.
                self.settings.set_tab_visible(Tab::Dns, true);
                self.tab = Tab::Dns;
                self.dns.set_page(DnsPage::Tls);
                self.tls.set_target(target);
                self.tls.set_server_name(server_name.unwrap_or_default());
                self.tls.start()
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

    /// Reads the saved scans once. The list is kept until a scan is saved.
    pub(super) fn ensure_saved_scans(&mut self) {
        if self.saved_scans.is_none() {
            self.saved_scans = Some(load_saved_scans());
        }
    }

    pub(super) fn show_summary(&mut self) {
        self.ensure_saved_scans();
        let scans = self.saved_scans.as_deref().unwrap_or_default();
        self.modal = Some(Modal::Summary(summarize(&self.hosts, scans, unix_now())));
    }

    pub(super) fn show_host_details(&mut self, key: String) {
        self.ensure_saved_scans();
        let scans = self.saved_scans.as_deref().unwrap_or_default();
        self.detail_scan_count = scans.len();
        self.detail_sightings = sightings(scans, &key);
        self.detail_timeline = device_timeline(scans, &key);
        self.note_draft = NoteDraft::from_note(self.notes.get(&key));
        self.modal = Some(Modal::HostDetails(key));
    }
}
