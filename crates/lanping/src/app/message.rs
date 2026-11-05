use super::*;

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

/// A field in the device notes form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteField {
    Owner,
    Location,
    Tags,
    Notes,
}

/// What is typed in the device notes form, before it is saved.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct NoteDraft {
    pub owner: String,
    pub location: String,
    pub tags: String,
    pub notes: String,
}

impl NoteDraft {
    pub(super) fn from_note(note: Option<&DeviceNote>) -> Self {
        note.map_or_else(Self::default, |note| Self {
            owner: note.owner.clone(),
            location: note.location.clone(),
            tags: note.tags_text(),
            notes: note.notes.clone(),
        })
    }

    pub(super) fn set(&mut self, field: NoteField, text: String) {
        match field {
            NoteField::Owner => self.owner = text,
            NoteField::Location => self.location = text,
            NoteField::Tags => self.tags = text,
            NoteField::Notes => self.notes = text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Modal {
    /// Menu for one device, identified by [`Host::device_key`].
    HostActions(String),
    SavePreset,
    Summary(Summary),
    /// Devices around this gateway, when it is known.
    NetworkMap(Option<std::net::Ipv4Addr>),
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
    ShowNetworkMap,
    ShowSummary,
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
    SnmpCommunityChanged(String),
    PortToggled(u16, bool),
    SelectAllPorts,
    SelectNoPorts,
    CustomPortsChanged(String),
    ColumnToggled(Column, bool),
    ShowColumnMenu(Column),
    ColumnDragStarted(Column),
    ColumnDraggedOver(Column),
    ColumnDragEnded,
    TabToggled(Tab, bool),
    TabDragStarted(Tab),
    TabDraggedOver(Tab),
    TabDragEnded,
    AutoRescanPicked(AutoInterval),
    NotifyToggled(bool),
    NotifyPortsToggled(bool),
    NotifyOfflineToggled(bool),
    IgnoreVirtualToggled(bool),
    CardViewToggled,
    LabelDraftChanged(String),
    NoteDraftChanged(NoteField, String),
    SaveNote(String),
    SaveLabel(String),
    RunHostAction(String, HostAction),
    OpenUrl(String),
    TabSelected(Tab),
    WifiRefresh,
    WifiAutoRefreshToggled(bool),
    WifiSortBy(WifiSortKey),
    WifiFilter(WifiFilterChange),
    WifiCopyRows,
    TraceTargetChanged(String),
    TraceStart,
    TraceStop,
    TraceRound(u64, Result<Vec<HopSample>, TraceError>),
    TraceCopyRows,
    SpeedPagePicked(SpeedPage),
    SpeedServicePicked(SpeedService),
    SpeedLocationPicked(LocationChoice),
    SpeedCountriesFound(Result<Vec<String>, String>),
    SpeedServersFound(u64, Result<Vec<Server>, String>),
    SpeedServerPicked(Server),
    SpeedServersSearchAgain,
    SpeedLanTargetChanged(String),
    SpeedLengthPicked(TestLength),
    SpeedKindPicked(TestKind),
    SpeedStart(Source),
    SpeedStop,
    SpeedEvent(u64, SpeedEvent),
    SpeedServerPortChanged(String),
    SpeedServerStart,
    SpeedServerStop,
    SpeedServerListening(u16, Vec<std::net::IpAddr>),
    SpeedServerFailed(String),
    SpeedServed(ServedStream),
    LatencyTargetChanged(String),
    LatencyIntervalPicked(Interval),
    LatencyStart,
    LatencyStop,
    LatencyReady(u64, Result<Prober, LatencyError>),
    LatencySample(u64, Option<f64>),
    LatencyCopySummary,
    LatencySaveLog,
    LatencyLogPathChosen(Option<PathBuf>),
    SnmpTargetChanged(String),
    SnmpStart,
    SnmpStop,
    SnmpDiscovered(u64, Result<Vec<Interface>, SnmpError>),
    SnmpPolled(u64, Result<Poll, SnmpError>),
    SnmpShowAllToggled(bool),
    TlsTargetChanged(String),
    TlsServerNameChanged(String),
    TlsCheck,
    TlsDone(u64, Result<TlsReport, TlsError>),
    NtpServerChanged(String),
    NtpCheck,
    NtpDone(u64, Result<NtpReport, NtpError>),
    UpnpRead,
    ServicesBrowse,
    ServicesToggle(String),
    ServicesDone(u64, Vec<ServiceEntry>),
    UpnpDone(u64, Result<UpnpReport, UpnpError>),
    DnsPagePicked(DnsPage),
    WhoisQueryChanged(String),
    WhoisLookup,
    WhoisDone(u64, Result<WhoisReport, WhoisError>),
    WhoisCopy,
    DnsNameChanged(String),
    DnsTypePicked(RecordType),
    DnsResolverPicked(ResolverChoice),
    DnsLookup,
    DnsLookupDone(u64, Result<Answer, DnsError>),
    DnsCompare,
    DnsComparisonDone(u64, Vec<ResolverTiming>),
    DnsCopyRecords,
    SubnetInputChanged(String),
    SubnetScanRange(String),
    WifiScanned(Result<Vec<AccessPoint>, WifiError>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notice {
    Info,
    Error,
}
