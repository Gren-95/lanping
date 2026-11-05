//! The Syslog tab state: a listener for log messages from other devices, the messages it
//! kept, and the filters on them.

use std::collections::VecDeque;
use std::fmt;
use std::net::IpAddr;

use iced::Task;
use iced::futures::SinkExt;
use iced::task::Handle;
use lanping_core::syslog::{self, Severity, SyslogMessage};
use lanping_core::text::contains_ignore_case;
use lanping_core::throughput::local_addresses;
use tokio::sync::mpsc;

use crate::app::Message;
use crate::job::StopOnDrop;
use crate::timestamp::format_unix_time_utc;

/// Messages kept; the oldest go first.
const KEPT_MESSAGES: usize = 5000;
/// Most rows drawn at once.
pub const SHOWN_MESSAGES: usize = 500;
const STREAM_BUFFER: usize = 16;
const CHANNEL_CAPACITY: usize = 1024;
/// Messages sent to the window together, so a busy device does not cause a redraw each.
const BATCH: usize = 256;

pub enum Status {
    Stopped,
    Starting,
    Listening {
        port: u16,
        tcp: bool,
        addresses: Vec<IpAddr>,
    },
    Failed(String),
}

/// A choice in the severity picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SeverityChoice {
    #[default]
    All,
    AtLeast(Severity),
}

impl SeverityChoice {
    pub fn all() -> Vec<Self> {
        std::iter::once(Self::All)
            .chain(
                Severity::ALL[..Severity::ALL.len() - 1]
                    .iter()
                    .copied()
                    .map(Self::AtLeast),
            )
            .collect()
    }
}

impl fmt::Display for SeverityChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::All => f.write_str("All severities"),
            Self::AtLeast(Severity::Emergency) => f.write_str("Emergency"),
            Self::AtLeast(severity) => write!(f, "{severity} and worse"),
        }
    }
}

/// What the lists on screen are narrowed to.
#[derive(Default)]
pub struct SyslogFilter {
    pub host: String,
    pub text: String,
    pub severity: SeverityChoice,
}

impl SyslogFilter {
    pub fn active(&self) -> bool {
        !self.host.trim().is_empty()
            || !self.text.trim().is_empty()
            || self.severity != SeverityChoice::All
    }
}

pub struct SyslogState {
    pub port: String,
    pub tcp: bool,
    pub status: Status,
    pub filter: SyslogFilter,
    /// Newest first.
    messages: VecDeque<SyslogMessage>,
    /// All received since the listener started, including those pushed out.
    pub received: u64,
    handle: Option<Handle>,
}

impl SyslogState {
    pub fn new() -> Self {
        Self {
            port: syslog::DEFAULT_PORT.to_string(),
            tcp: false,
            status: Status::Stopped,
            filter: SyslogFilter::default(),
            messages: VecDeque::new(),
            received: 0,
            handle: None,
        }
    }

    pub fn listening(&self) -> bool {
        matches!(self.status, Status::Starting | Status::Listening { .. })
    }

    pub fn start(&mut self) -> Task<Message> {
        let Some(port) = self
            .port
            .trim()
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
        else {
            self.status = Status::Failed("The port must be a number from 1 to 65535".to_owned());
            return Task::none();
        };
        self.stop();
        self.status = Status::Starting;
        self.received = 0;
        let (task, handle) = Task::stream(listen_stream(port, self.tcp)).abortable();
        self.handle = Some(handle);
        task
    }

    pub fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        self.status = Status::Stopped;
    }

    pub fn is_listening_now(&mut self, port: u16, tcp: bool, addresses: Vec<IpAddr>) {
        self.status = Status::Listening {
            port,
            tcp,
            addresses,
        };
    }

    pub fn failed(&mut self, reason: String) {
        self.handle = None;
        self.status = Status::Failed(reason);
    }

    pub fn receive(&mut self, batch: Vec<SyslogMessage>) {
        if !self.listening() {
            return;
        }
        self.received += batch.len() as u64;
        for message in batch {
            self.messages.push_front(message);
        }
        self.messages.truncate(KEPT_MESSAGES);
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    pub fn kept(&self) -> usize {
        self.messages.len()
    }

    /// The newest messages that pass the filter, up to `limit`.
    pub fn shown(&self, limit: usize) -> Vec<&SyslogMessage> {
        let host = self.filter.host.trim().to_lowercase();
        let text = self.filter.text.trim().to_lowercase();
        self.messages
            .iter()
            .filter(|message| passes(message, &host, &text, self.filter.severity))
            .take(limit)
            .collect()
    }

    /// The messages that pass the filter as lines of raw text, newest first.
    pub fn shown_as_text(&self) -> (String, usize) {
        let shown = self.shown(KEPT_MESSAGES);
        let lines: Vec<String> = shown
            .iter()
            .map(|message| {
                format!(
                    "{}\t{}\t{}",
                    format_unix_time_utc(message.received_at_unix),
                    message.source,
                    message.raw
                )
            })
            .collect();
        (lines.join("\n"), shown.len())
    }
}

/// Whether `message` passes the filter. `host` and `text` are lowercase.
fn passes(message: &SyslogMessage, host: &str, text: &str, severity: SeverityChoice) -> bool {
    if let SeverityChoice::AtLeast(limit) = severity
        && !message.severity.is_some_and(|own| own <= limit)
    {
        return false;
    }
    let host_ok = host.is_empty()
        || message
            .hostname
            .as_deref()
            .is_some_and(|name| contains_ignore_case(name, host))
        || contains_ignore_case(&message.source.to_string(), host);
    let text_ok = text.is_empty()
        || contains_ignore_case(&message.text, text)
        || message
            .app
            .as_deref()
            .is_some_and(|app| contains_ignore_case(app, text));
    host_ok && text_ok
}

/// Listens until the stream is dropped, reporting what it receives in batches.
fn listen_stream(port: u16, tcp: bool) -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(STREAM_BUFFER, async move |mut output| {
        let listening = match syslog::bind(port, tcp).await {
            Ok(listening) => listening,
            Err(error) => {
                let reason = format!("Could not listen on port {port}: {error}");
                let _ = output.send(Message::SyslogFailed(reason)).await;
                return;
            }
        };
        let actual = listening.port;
        let _ = output
            .send(Message::SyslogListening(actual, tcp, local_addresses()))
            .await;
        let (sink, mut incoming) = mpsc::channel(CHANNEL_CAPACITY);
        let _listener = StopOnDrop(tokio::spawn(listening.run(sink)));
        let mut batch = Vec::new();
        while incoming.recv_many(&mut batch, BATCH).await > 0 {
            if output
                .send(Message::SyslogReceived(std::mem::take(&mut batch)))
                .await
                .is_err()
            {
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn message(host: &str, severity: Severity, text: &str) -> SyslogMessage {
        SyslogMessage {
            source: IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7)),
            received_at_unix: 0,
            facility: Some("daemon"),
            severity: Some(severity),
            timestamp: None,
            hostname: Some(host.to_owned()),
            app: Some("sshd".to_owned()),
            text: text.to_owned(),
            raw: text.to_owned(),
        }
    }

    fn listening_state() -> SyslogState {
        let mut state = SyslogState::new();
        state.status = Status::Listening {
            port: 5514,
            tcp: false,
            addresses: Vec::new(),
        };
        state
    }

    #[test]
    fn the_newest_message_comes_first_and_the_oldest_fall_off() {
        let mut state = listening_state();
        state.receive(vec![
            message("a", Severity::Info, "one"),
            message("a", Severity::Info, "two"),
        ]);
        assert_eq!(state.shown(10)[0].text, "two");
        state.receive(
            (0..KEPT_MESSAGES + 5)
                .map(|i| message("a", Severity::Info, &i.to_string()))
                .collect(),
        );
        assert_eq!(state.kept(), KEPT_MESSAGES);
        assert_eq!(state.received, KEPT_MESSAGES as u64 + 7);
    }

    #[test]
    fn messages_that_arrive_after_stopping_are_ignored() {
        let mut state = SyslogState::new();
        state.receive(vec![message("a", Severity::Info, "late")]);
        assert_eq!(state.kept(), 0);
    }

    #[test]
    fn the_filters_narrow_by_host_text_and_severity() {
        let mut state = listening_state();
        state.receive(vec![
            message("router", Severity::Error, "link down"),
            message("nas", Severity::Info, "backup done"),
            message("router", Severity::Debug, "poll"),
        ]);
        state.filter.host = " ROUTER ".to_owned();
        assert_eq!(state.shown(10).len(), 2);
        state.filter.severity = SeverityChoice::AtLeast(Severity::Warning);
        assert_eq!(state.shown(10).len(), 1);
        state.filter = SyslogFilter {
            text: "BACKUP".to_owned(),
            ..SyslogFilter::default()
        };
        assert_eq!(state.shown(10)[0].hostname.as_deref(), Some("nas"));
        state.filter = SyslogFilter {
            host: "10.0.0".to_owned(),
            ..SyslogFilter::default()
        };
        assert_eq!(state.shown(10).len(), 3, "the address counts as a host");
        assert!(state.filter.active());
        assert!(!SyslogFilter::default().active());
    }

    #[test]
    fn a_severity_filter_leaves_out_messages_with_no_severity() {
        let mut unknown = message("x", Severity::Info, "no priority");
        unknown.severity = None;
        let mut state = listening_state();
        state.receive(vec![unknown]);
        assert_eq!(state.shown(10).len(), 1);
        state.filter.severity = SeverityChoice::AtLeast(Severity::Debug);
        assert!(state.shown(10).is_empty());
    }

    #[test]
    fn only_so_many_rows_are_drawn() {
        let mut state = listening_state();
        state.receive(
            (0..20)
                .map(|i| message("a", Severity::Info, &i.to_string()))
                .collect(),
        );
        assert_eq!(state.shown(5).len(), 5);
        assert_eq!(state.shown_as_text().1, 20);
    }

    #[test]
    fn a_bad_port_is_refused_before_listening() {
        let mut state = SyslogState::new();
        for bad in ["", "0", "x", "70000"] {
            state.port = bad.to_owned();
            let _ = state.start();
            assert!(matches!(state.status, Status::Failed(_)), "{bad:?}");
            assert!(!state.listening());
        }
    }

    #[test]
    fn the_severity_picker_lists_all_then_each_level_but_the_last() {
        let choices = SeverityChoice::all();
        assert_eq!(choices[0].to_string(), "All severities");
        assert_eq!(choices[1].to_string(), "Emergency");
        assert_eq!(choices[6].to_string(), "Notice and worse");
        assert_eq!(choices.len(), 8);
    }
}
