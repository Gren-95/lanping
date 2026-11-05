use super::*;

impl LanPing {
    pub(super) fn ask_export_path(&mut self, format: ExportFormat) -> Task<Message> {
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

    /// Saves finished tests or runs next to the saved scans and refreshes the history list.
    pub(crate) fn save_measurements(&mut self, measurements: &[Measurement]) {
        let Some(directory) = measurements_directory() else {
            return;
        };
        if measurements.is_empty() {
            return;
        }
        for measurement in measurements {
            if let Err(error) = save_measurement(&directory, measurement) {
                self.set_notice(Notice::Error, format!("Could not save the result: {error}"));
                return;
            }
        }
        self.speed.refresh_history(Some(&directory));
    }

    pub(super) fn save_latency_log(&mut self, path: &Path) {
        match fs::write(path, self.latency.log_as_csv()) {
            Ok(()) => self.set_notice(Notice::Info, format!("Saved {}", path.display())),
            Err(error) => self.set_notice(Notice::Error, format!("Could not save log: {error}")),
        }
    }

    pub(super) fn export(&mut self, format: ExportFormat, path: &Path) {
        let generated_at = format_unix_utc(unix_now());
        let input = ReportInput {
            target: &self.target,
            generated_at: &generated_at,
            hosts: &self.hosts,
            labels: &self.labels,
            notes: &self.notes,
        };
        let result = format
            .render(&input)
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
        self.saved_scans = None;
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
        self.history = list_scan_summaries(&directory)
            .into_iter()
            .map(|(path, scan)| HistoryEntry {
                path,
                label: format!(
                    "{} - {} ({} hosts)",
                    format_unix_utc(scan.finished_at_unix),
                    scan.target,
                    scan.host_count
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
    pub(super) fn open_scan_file(&mut self, path: PathBuf) {
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

    pub(super) fn load_history(&mut self, entry: &HistoryEntry) {
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
}
