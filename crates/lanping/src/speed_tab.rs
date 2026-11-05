//! The Speed tab: internet and LAN throughput tests, the test server, and saved results.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use iced::Task;
use iced::futures::SinkExt;
use iced::task::Handle;
use lanping_core::measurements::{Measurement, ThroughputRecord, list};
use lanping_core::ndt7::{self, Server};
use lanping_core::speedtest::{self, SERVICE_HOST};
use lanping_core::throughput::{
    self, DEFAULT_PORT, Direction, ServedStream, ThroughputResult, run_client,
};
use lanping_core::trace::validate_target;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::app::Message;
use crate::speed_locations::LocationChoice;

const STREAM_BUFFER: usize = 64;
/// Served streams of one test arrive together; ones this close are shown as one test.
const SAME_TEST_WINDOW: Duration = Duration::from_secs(3);
const SHOWN_SERVED_TESTS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeedPage {
    #[default]
    Internet,
    Lan,
    History,
}

impl SpeedPage {
    pub const ALL: [Self; 3] = [Self::Internet, Self::Lan, Self::History];

    pub fn title(self) -> &'static str {
        match self {
            Self::Internet => "Internet",
            Self::Lan => "LAN",
            Self::History => "History",
        }
    }
}

/// What a test is measured against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Cloudflare,
    Mlab,
    Lan,
}

/// The service the Internet page tests against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpeedService {
    #[default]
    Cloudflare,
    Mlab,
}

impl SpeedService {
    pub const ALL: [Self; 2] = [Self::Cloudflare, Self::Mlab];

    pub fn source(self) -> Source {
        match self {
            Self::Cloudflare => Source::Cloudflare,
            Self::Mlab => Source::Mlab,
        }
    }
}

impl std::fmt::Display for SpeedService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Cloudflare => "Cloudflare",
            Self::Mlab => "M-Lab (servers worldwide)",
        })
    }
}

/// Where a started test runs, with what it needs to get there.
enum Target {
    Cloudflare,
    Mlab {
        location: ndt7::Location,
        machine: String,
    },
    Lan {
        host: String,
        port: u16,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TestLength {
    Short,
    #[default]
    Normal,
    Long,
}

impl TestLength {
    pub const ALL: [Self; 3] = [Self::Short, Self::Normal, Self::Long];

    pub fn seconds(self) -> u8 {
        match self {
            Self::Short => 5,
            Self::Normal => 10,
            Self::Long => 20,
        }
    }
}

impl std::fmt::Display for TestLength {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} seconds", self.seconds())
    }
}

pub enum Run {
    Idle,
    Running {
        place: String,
        direction: Direction,
        /// Megabits per second of each finished second so far.
        samples: Vec<f64>,
    },
    Done(ThroughputRecord),
    Failed(String),
}

pub enum ServerStatus {
    Stopped,
    Starting,
    Listening { port: u16, addresses: Vec<IpAddr> },
    Failed(String),
}

/// A test the server took part in.
pub struct ServedTest {
    pub peer: IpAddr,
    pub direction: Direction,
    pub bytes: u64,
    last_seen: Instant,
}

pub struct SpeedState {
    pub page: SpeedPage,
    pub service: SpeedService,
    pub location: LocationChoice,
    /// Servers found for the chosen location. Their addresses carry a token that expires, so a
    /// test asks again for the chosen server just before it starts.
    pub servers: Vec<Server>,
    pub selected_server: Option<Server>,
    pub locating: bool,
    pub locate_error: Option<String>,
    locate_generation: u64,
    pub lan_target: String,
    pub length: TestLength,
    pub run: Run,
    pub server_port: String,
    pub server: ServerStatus,
    pub served: Vec<ServedTest>,
    pub history: Vec<(PathBuf, Measurement)>,
    /// Counts tests, so an answer from a stopped one is ignored.
    generation: u64,
    handle: Option<Handle>,
    server_handle: Option<Handle>,
}

/// Stops the task it holds when dropped, so a stopped stream does not leave work running.
struct StopOnDrop<T>(JoinHandle<T>);

impl<T> Drop for StopOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

/// `host` or `host:port` of a machine that runs the test server.
pub fn parse_lan_target(text: &str) -> Result<(String, u16), String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter the address of the machine that runs the test server".to_owned());
    }
    let (host, port) = match text.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| "The port must be a number from 1 to 65535".to_owned())?,
        ),
        None => (text, DEFAULT_PORT),
    };
    let host = validate_target(host).map_err(|_| "That is not a valid address".to_owned())?;
    Ok((host.to_owned(), port))
}

fn parse_server_port(text: &str) -> Result<u16, String> {
    text.trim()
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
        .ok_or_else(|| "The port must be a number from 1 to 65535".to_owned())
}

impl SpeedState {
    pub fn new(history_directory: Option<&std::path::Path>) -> Self {
        Self {
            page: SpeedPage::default(),
            service: SpeedService::default(),
            location: LocationChoice::default(),
            servers: Vec::new(),
            selected_server: None,
            locating: false,
            locate_error: None,
            locate_generation: 0,
            lan_target: String::new(),
            length: TestLength::default(),
            run: Run::Idle,
            server_port: DEFAULT_PORT.to_string(),
            server: ServerStatus::Stopped,
            served: Vec::new(),
            history: history_directory.map(list).unwrap_or_default(),
            generation: 0,
            handle: None,
            server_handle: None,
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.run, Run::Running { .. })
    }

    pub fn refresh_history(&mut self, directory: Option<&std::path::Path>) {
        self.history = directory.map(list).unwrap_or_default();
    }

    /// Chooses the service. M-Lab looks for servers when none are known yet.
    pub fn set_service(&mut self, service: SpeedService) -> Task<Message> {
        self.service = service;
        if service == SpeedService::Mlab && self.servers.is_empty() && !self.locating {
            return self.locate();
        }
        Task::none()
    }

    /// Chooses where to look for M-Lab servers, and looks.
    pub fn set_location(&mut self, location: LocationChoice) -> Task<Message> {
        self.location = location;
        self.servers.clear();
        self.selected_server = None;
        self.locate()
    }

    fn locate(&mut self) -> Task<Message> {
        self.locate_generation += 1;
        self.locating = true;
        self.locate_error = None;
        let generation = self.locate_generation;
        let location = self.location.location();
        Task::perform(
            async move { ndt7::locate(&location).await },
            move |result| Message::SpeedServersFound(generation, result),
        )
    }

    pub fn servers_found(&mut self, generation: u64, result: Result<Vec<Server>, String>) {
        if generation != self.locate_generation {
            return;
        }
        self.locating = false;
        match result {
            Ok(servers) => {
                let kept = self.selected_server.as_ref().and_then(|chosen| {
                    servers
                        .iter()
                        .find(|server| server.machine == chosen.machine)
                        .cloned()
                });
                self.selected_server = kept.or_else(|| servers.first().cloned());
                self.servers = servers;
            }
            Err(message) => {
                self.servers.clear();
                self.selected_server = None;
                self.locate_error = Some(message);
            }
        }
    }

    pub fn set_server(&mut self, server: Server) {
        self.selected_server = Some(server);
    }

    /// Starts a test. A missing server or a bad LAN address shows as a failed run.
    pub fn start_test(&mut self, source: Source, direction: Direction) -> Task<Message> {
        if self.running() {
            return Task::none();
        }
        let (target, place) = match source {
            Source::Cloudflare => (Target::Cloudflare, format!("Internet ({SERVICE_HOST})")),
            Source::Mlab => match &self.selected_server {
                Some(server) => (
                    Target::Mlab {
                        location: self.location.location(),
                        machine: server.machine.clone(),
                    },
                    format!("Internet (M-Lab, {})", server.place()),
                ),
                None => {
                    self.run = Run::Failed("Pick an M-Lab server first".to_owned());
                    return Task::none();
                }
            },
            Source::Lan => match parse_lan_target(&self.lan_target) {
                Ok((host, port)) => {
                    let place = format!("LAN ({host}:{port})");
                    (Target::Lan { host, port }, place)
                }
                Err(message) => {
                    self.run = Run::Failed(message);
                    return Task::none();
                }
            },
        };
        self.generation += 1;
        self.run = Run::Running {
            place,
            direction,
            samples: Vec::new(),
        };
        let stream = test_stream(target, direction, self.length.seconds(), self.generation);
        let (task, handle) = Task::stream(stream).abortable();
        self.handle = Some(handle);
        task
    }

    pub fn stop_test(&mut self) {
        self.generation += 1;
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        if self.running() {
            self.run = Run::Idle;
        }
    }

    pub fn progress(&mut self, generation: u64, mbps: f64) {
        if generation != self.generation {
            return;
        }
        if let Run::Running { samples, .. } = &mut self.run {
            samples.push(mbps);
        }
    }

    /// Records the end of a test. A good result comes back as a record to save.
    pub fn finish(
        &mut self,
        generation: u64,
        result: Result<ThroughputResult, String>,
    ) -> Option<Measurement> {
        if generation != self.generation {
            return None;
        }
        self.handle = None;
        let Run::Running { place, .. } = std::mem::replace(&mut self.run, Run::Idle) else {
            return None;
        };
        match result {
            Ok(result) => {
                let record = ThroughputRecord {
                    place,
                    finished_at_unix: unix_seconds(),
                    result,
                };
                self.run = Run::Done(record.clone());
                Some(Measurement::Throughput(record))
            }
            Err(message) => {
                self.run = Run::Failed(message);
                None
            }
        }
    }

    pub fn start_server(&mut self) -> Task<Message> {
        let port = match parse_server_port(&self.server_port) {
            Ok(port) => port,
            Err(message) => {
                self.server = ServerStatus::Failed(message);
                return Task::none();
            }
        };
        self.stop_server();
        self.server = ServerStatus::Starting;
        let (task, handle) = Task::stream(server_stream(port)).abortable();
        self.server_handle = Some(handle);
        task
    }

    pub fn stop_server(&mut self) {
        if let Some(handle) = self.server_handle.take() {
            handle.abort();
        }
        self.server = ServerStatus::Stopped;
    }

    pub fn server_listening(&mut self, port: u16, addresses: Vec<IpAddr>) {
        self.server = ServerStatus::Listening { port, addresses };
    }

    pub fn server_failed(&mut self, message: String) {
        self.server_handle = None;
        self.server = ServerStatus::Failed(message);
    }

    /// Adds a served stream; streams of one test that arrive together make one entry.
    pub fn served(&mut self, stream: ServedStream) {
        let now = Instant::now();
        let peer = stream.peer.ip();
        let same_test = self.served.iter_mut().find(|test| {
            test.peer == peer
                && test.direction == stream.direction
                && now.duration_since(test.last_seen) < SAME_TEST_WINDOW
        });
        match same_test {
            Some(test) => {
                test.bytes += stream.bytes;
                test.last_seen = now;
            }
            None => {
                self.served.insert(
                    0,
                    ServedTest {
                        peer,
                        direction: stream.direction,
                        bytes: stream.bytes,
                        last_seen: now,
                    },
                );
                self.served.truncate(SHOWN_SERVED_TESTS);
            }
        }
    }
}

/// Runs a test and reports each second, then the result.
fn test_stream(
    target: Target,
    direction: Direction,
    seconds: u8,
    generation: u64,
) -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(STREAM_BUFFER, async move |mut output| {
        let (progress, mut ticks) = mpsc::unbounded_channel();
        let mut run = StopOnDrop(tokio::spawn(async move {
            match target {
                Target::Cloudflare => speedtest::run(direction, seconds, Some(progress)).await,
                Target::Lan { host, port } => {
                    run_client(&host, port, direction, seconds, Some(progress)).await
                }
                Target::Mlab { location, machine } => {
                    run_mlab(&location, &machine, direction, seconds, progress).await
                }
            }
        }));
        while let Some(mbps) = ticks.recv().await {
            if output
                .send(Message::SpeedProgress(generation, mbps))
                .await
                .is_err()
            {
                return;
            }
        }
        let result = (&mut run.0)
            .await
            .unwrap_or_else(|error| Err(error.to_string()));
        let _ = output
            .send(Message::SpeedFinished(generation, result))
            .await;
    })
}

/// Asks M-Lab for fresh server addresses, since the ones found earlier expire, and tests
/// against the chosen machine, or the first one offered if it is gone.
async fn run_mlab(
    location: &ndt7::Location,
    machine: &str,
    direction: Direction,
    seconds: u8,
    progress: mpsc::UnboundedSender<f64>,
) -> Result<ThroughputResult, String> {
    let servers = ndt7::locate(location).await?;
    let server = servers
        .iter()
        .find(|server| server.machine == machine)
        .or(servers.first())
        .ok_or_else(|| "M-Lab has no test server there".to_owned())?;
    ndt7::run(server, direction, seconds, Some(progress)).await
}

/// Listens for tests and reports each stream it serves, until stopped.
fn server_stream(port: u16) -> impl iced::futures::Stream<Item = Message> {
    iced::stream::channel(STREAM_BUFFER, async move |mut output| {
        let listener = match throughput::bind(port).await {
            Ok(listener) => listener,
            Err(error) => {
                let message = format!("Could not listen on port {port}: {error}");
                let _ = output.send(Message::SpeedServerFailed(message)).await;
                return;
            }
        };
        let actual_port = listener.local_addr().map_or(port, |address| address.port());
        let _ = output
            .send(Message::SpeedServerListening(
                actual_port,
                throughput::local_addresses(),
            ))
            .await;
        let (served, mut events) = mpsc::unbounded_channel();
        let _server = StopOnDrop(tokio::spawn(throughput::serve(listener, served)));
        while let Some(stream) = events.recv().await {
            if output.send(Message::SpeedServed(stream)).await.is_err() {
                return;
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result() -> ThroughputResult {
        ThroughputResult::new(Direction::Download, 1_000_000, 1.0, vec![8.0])
    }

    fn running_state() -> SpeedState {
        let mut state = SpeedState::new(None);
        state.generation = 1;
        state.run = Run::Running {
            place: "LAN (x)".to_owned(),
            direction: Direction::Download,
            samples: Vec::new(),
        };
        state
    }

    #[test]
    fn reads_a_lan_target_with_and_without_a_port() {
        assert_eq!(
            parse_lan_target("192.168.0.5"),
            Ok(("192.168.0.5".to_owned(), DEFAULT_PORT))
        );
        assert_eq!(
            parse_lan_target(" nas.lan:6000 "),
            Ok(("nas.lan".to_owned(), 6000))
        );
    }

    #[test]
    fn refuses_bad_lan_targets() {
        for target in ["", "  ", "nas.lan:0", "nas.lan:x", "-x", "a b"] {
            assert!(parse_lan_target(target).is_err(), "{target}");
        }
    }

    #[test]
    fn a_bad_lan_target_is_a_failed_run_not_a_started_test() {
        let mut state = SpeedState::new(None);
        let _ = state.start_test(Source::Lan, Direction::Download);
        assert!(matches!(state.run, Run::Failed(_)));
        assert!(!state.running());
    }

    #[test]
    fn progress_and_the_result_follow_the_running_test() {
        let mut state = running_state();
        state.progress(1, 100.0);
        state.progress(9, 5.0);
        let Run::Running { samples, .. } = &state.run else {
            panic!("expected a running test");
        };
        assert_eq!(samples, &[100.0]);
        let saved = state.finish(1, Ok(result()));
        assert!(matches!(saved, Some(Measurement::Throughput(_))));
        let Run::Done(record) = &state.run else {
            panic!("expected a finished test");
        };
        assert_eq!(record.place, "LAN (x)");
    }

    #[test]
    fn an_answer_from_a_stopped_test_is_ignored() {
        let mut state = running_state();
        state.stop_test();
        assert!(state.finish(1, Ok(result())).is_none());
        assert!(matches!(state.run, Run::Idle));
    }

    #[test]
    fn a_failed_test_shows_its_message_and_saves_nothing() {
        let mut state = running_state();
        assert!(state.finish(1, Err("no route".to_owned())).is_none());
        assert!(matches!(state.run, Run::Failed(ref message) if message == "no route"));
    }

    #[test]
    fn streams_of_one_test_make_one_served_entry() {
        let mut state = SpeedState::new(None);
        let stream = |bytes| ServedStream {
            peer: "192.168.0.7:5000".parse().unwrap(),
            direction: Direction::Upload,
            bytes,
        };
        state.served(stream(100));
        state.served(stream(50));
        assert_eq!(state.served.len(), 1);
        assert_eq!(state.served[0].bytes, 150);
    }

    fn server(machine: &str) -> Server {
        Server::new(machine, "City", "XX", "wss://h/d", "wss://h/u")
    }

    #[test]
    fn found_servers_select_the_first_and_keep_a_still_listed_choice() {
        let mut state = SpeedState::new(None);
        state.locate_generation = 1;
        state.servers_found(1, Ok(vec![server("a.example"), server("b.example")]));
        assert_eq!(state.selected_server.as_ref().unwrap().machine, "a.example");
        state.set_server(server("b.example"));
        state.locate_generation = 2;
        state.servers_found(2, Ok(vec![server("a.example"), server("b.example")]));
        assert_eq!(state.selected_server.as_ref().unwrap().machine, "b.example");
        state.locate_generation = 3;
        state.servers_found(3, Ok(vec![server("c.example")]));
        assert_eq!(state.selected_server.as_ref().unwrap().machine, "c.example");
    }

    #[test]
    fn a_failed_search_clears_the_servers_and_keeps_the_reason() {
        let mut state = SpeedState::new(None);
        state.locate_generation = 1;
        state.servers_found(1, Ok(vec![server("a.example")]));
        state.locate_generation = 2;
        state.servers_found(2, Err("M-Lab has no test server there".to_owned()));
        assert!(state.servers.is_empty() && state.selected_server.is_none());
        assert_eq!(
            state.locate_error.as_deref(),
            Some("M-Lab has no test server there")
        );
    }

    #[test]
    fn an_answer_for_an_older_search_is_ignored() {
        let mut state = SpeedState::new(None);
        state.locate_generation = 5;
        state.servers_found(4, Ok(vec![server("old.example")]));
        assert!(state.servers.is_empty());
    }

    #[test]
    fn an_mlab_test_needs_a_chosen_server() {
        let mut state = SpeedState::new(None);
        let _ = state.start_test(Source::Mlab, Direction::Download);
        assert!(matches!(state.run, Run::Failed(ref message) if message.contains("server")));
    }

    #[test]
    fn a_bad_server_port_is_an_error_and_nothing_listens() {
        let mut state = SpeedState::new(None);
        state.server_port = "0".to_owned();
        let _ = state.start_server();
        assert!(matches!(state.server, ServerStatus::Failed(_)));
    }

    /// Runs `future` on a small runtime, as the app's task runner would.
    fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    #[test]
    fn a_lan_test_runs_against_the_server_stream_and_reports_each_step() {
        use iced::futures::StreamExt;
        block_on(async {
            let mut server = Box::pin(server_stream(0));
            let Some(Message::SpeedServerListening(port, _)) = server.next().await else {
                panic!("the server should say it is listening first");
            };
            let target = Target::Lan {
                host: "127.0.0.1".to_owned(),
                port,
            };
            let mut test = Box::pin(test_stream(target, Direction::Download, 2, 7));
            let mut ticks = 0;
            let result = loop {
                match test.next().await {
                    Some(Message::SpeedProgress(7, _)) => ticks += 1,
                    Some(Message::SpeedFinished(7, result)) => break result,
                    other => panic!("unexpected message {other:?}"),
                }
            };
            let result = result.unwrap();
            assert!(result.mbps > 0.0 && ticks >= 1);
            let Some(Message::SpeedServed(stream)) = server.next().await else {
                panic!("the server should report the stream it served");
            };
            assert_eq!(stream.direction, Direction::Download);
        });
    }

    #[test]
    fn the_server_stream_reports_a_port_that_is_taken() {
        use iced::futures::StreamExt;
        block_on(async {
            let taken = throughput::bind(0).await.unwrap();
            let port = taken.local_addr().unwrap().port();
            let mut server = Box::pin(server_stream(port));
            let Some(Message::SpeedServerFailed(message)) = server.next().await else {
                panic!("binding a taken port should fail");
            };
            assert!(message.contains(&port.to_string()), "{message}");
        });
    }
}
