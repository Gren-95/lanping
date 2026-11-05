//! The Speed tab: internet and LAN throughput tests, the test server, and saved results.

use std::net::IpAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::Task;
use iced::task::Handle;
use lanping_core::measurements::{Measurement, list};
use lanping_core::ndt7::{self, Server};
use lanping_core::speedtest::SERVICE_HOST;
use lanping_core::throughput::{DEFAULT_PORT, ServedStream};
use lanping_core::trace::validate_target;

use crate::app::Message;
use crate::speed_locations::LocationChoice;
use crate::speed_model::*;
use crate::speed_run::{Target, mlab_place, server_stream, test_stream};

/// Served streams of one test arrive together; ones this close are shown as one test.
const SAME_TEST_WINDOW: Duration = Duration::from_secs(3);
const SHOWN_SERVED_TESTS: usize = 5;

pub struct SpeedState {
    pub page: SpeedPage,
    pub service: SpeedService,
    pub location: LocationChoice,
    /// The places M-Lab can be asked for: the known list at first, then M-Lab's own.
    pub countries: Vec<LocationChoice>,
    countries_loaded: bool,
    /// Servers found for the chosen location. Their addresses carry a token that expires, so a
    /// test asks again for the chosen server just before it starts.
    pub servers: Vec<Server>,
    pub selected_server: Option<Server>,
    pub locating: bool,
    pub locate_error: Option<String>,
    locate_generation: u64,
    /// When `servers` was fetched: their addresses stop working after about a minute.
    servers_fetched_at: Option<Instant>,
    pub lan_target: String,
    pub length: TestLength,
    pub kind: TestKind,
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
            countries: LocationChoice::known(),
            countries_loaded: false,
            servers: Vec::new(),
            selected_server: None,
            locating: false,
            locate_error: None,
            locate_generation: 0,
            servers_fetched_at: None,
            lan_target: String::new(),
            length: TestLength::default(),
            kind: TestKind::default(),
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
        if service != SpeedService::Mlab {
            return Task::none();
        }
        let mut tasks = Vec::new();
        if self.servers.is_empty() && !self.locating {
            tasks.push(self.locate());
        }
        if !self.countries_loaded {
            self.countries_loaded = true;
            tasks.push(Task::perform(
                async { ndt7::countries().await },
                Message::SpeedCountriesFound,
            ));
        }
        Task::batch(tasks)
    }

    /// Replaces the known list with M-Lab's own. A failure keeps the known list.
    pub fn countries_found(&mut self, result: Result<Vec<String>, String>) {
        let Ok(codes) = result else {
            return;
        };
        self.countries = LocationChoice::from_codes(&codes);
        if !self.countries.contains(&self.location) {
            self.location = LocationChoice::nearest();
        }
    }

    /// Chooses where to look for M-Lab servers, and looks.
    pub fn set_location(&mut self, location: LocationChoice) -> Task<Message> {
        self.location = location;
        self.servers.clear();
        self.selected_server = None;
        self.locate()
    }

    /// Looks for servers again, for example after M-Lab said it was busy.
    pub fn search_again(&mut self) -> Task<Message> {
        if self.locating {
            return Task::none();
        }
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
                self.servers_fetched_at = Some(Instant::now());
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

    /// Whether the found servers can be used as they are, which saves asking M-Lab again.
    fn server_addresses_are_fresh(&self) -> bool {
        self.servers_fetched_at
            .is_some_and(|fetched| fetched.elapsed() < ndt7::SERVER_ADDRESS_LIFETIME)
    }

    /// Starts a test of the chosen kind. A missing server or a bad LAN address shows as a failed run.
    pub fn start_test(&mut self, source: Source) -> Task<Message> {
        if self.running() {
            return Task::none();
        }
        let (target, place) = match source {
            Source::Cloudflare => (Target::Cloudflare, format!("Internet ({SERVICE_HOST})")),
            Source::Mlab => match &self.selected_server {
                Some(server) => {
                    let target = if self.server_addresses_are_fresh() {
                        Target::MlabFresh(server.clone())
                    } else {
                        Target::Mlab {
                            location: self.location.location(),
                            machine: server.machine.clone(),
                        }
                    };
                    (target, mlab_place(server))
                }
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
            phase: Phase::Ping,
            samples: Vec::new(),
            report: Report::new(place),
        };
        let stream = test_stream(target, self.kind, self.length.seconds(), self.generation);
        let (task, handle) = Task::stream(stream).abortable();
        self.handle = Some(handle);
        task
    }

    /// Stops a running test. Steps that had finished come back to be saved.
    pub fn stop_test(&mut self) -> Vec<Measurement> {
        self.generation += 1;
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        match std::mem::replace(&mut self.run, Run::Idle) {
            Run::Running { report, .. } => report.records(),
            other => {
                self.run = other;
                Vec::new()
            }
        }
    }

    /// Applies one event of the running test. When the test ends, the steps that finished
    /// come back to be saved.
    pub fn event(&mut self, generation: u64, event: SpeedEvent) -> Vec<Measurement> {
        if generation != self.generation {
            return Vec::new();
        }
        let Run::Running {
            phase,
            samples,
            report,
        } = &mut self.run
        else {
            return Vec::new();
        };
        match event {
            SpeedEvent::Place(place) => report.place = place,
            SpeedEvent::Phase(next) => {
                *phase = next;
                samples.clear();
            }
            SpeedEvent::Progress(mbps) => samples.push(mbps),
            SpeedEvent::Ping(ping) => report.ping = Some(ping),
            SpeedEvent::PingFailed(message) => report.ping_error = Some(message),
            SpeedEvent::Result(result) => report.results.push(result),
            SpeedEvent::Failed(message) => {
                report.error = Some(message);
                return self.finish();
            }
            SpeedEvent::Finished => return self.finish(),
        }
        Vec::new()
    }

    fn finish(&mut self) -> Vec<Measurement> {
        self.handle = None;
        let Run::Running { report, .. } = std::mem::replace(&mut self.run, Run::Idle) else {
            return Vec::new();
        };
        let records = report.records();
        self.run = Run::Done(report);
        records
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

#[cfg(test)]
mod tests {
    use super::*;
    use lanping_core::ping::PingSummary;
    use lanping_core::rtt_stats::RttStats;
    use lanping_core::throughput::{self, Direction, ThroughputResult};

    fn result(direction: Direction) -> ThroughputResult {
        ThroughputResult::new(direction, 1_000_000, 1.0, vec![8.0])
    }

    fn ping() -> PingSummary {
        let mut stats = RttStats::new(3);
        stats.record(Some(12.0));
        PingSummary::from_stats(&stats)
    }

    fn running_state() -> SpeedState {
        let mut state = SpeedState::new(None);
        state.generation = 1;
        state.run = Run::Running {
            phase: Phase::Ping,
            samples: Vec::new(),
            report: Report::new("LAN (x)".to_owned()),
        };
        state
    }

    fn server(machine: &str) -> Server {
        Server::new(machine, "City", "XX", "wss://h/d", "wss://h/u")
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
    fn a_test_does_download_and_upload_unless_told_otherwise() {
        assert_eq!(TestKind::default(), TestKind::Both);
        assert_eq!(
            TestKind::Both.directions(),
            [Direction::Download, Direction::Upload]
        );
        assert_eq!(TestKind::DownloadOnly.directions(), [Direction::Download]);
        assert_eq!(TestKind::UploadOnly.directions(), [Direction::Upload]);
    }

    #[test]
    fn a_bad_lan_target_is_a_failed_run_not_a_started_test() {
        let mut state = SpeedState::new(None);
        let _ = state.start_test(Source::Lan);
        assert!(matches!(state.run, Run::Failed(_)));
        assert!(!state.running());
    }

    #[test]
    fn an_mlab_test_needs_a_chosen_server() {
        let mut state = SpeedState::new(None);
        let _ = state.start_test(Source::Mlab);
        assert!(matches!(state.run, Run::Failed(ref message) if message.contains("server")));
    }

    #[test]
    fn events_build_up_the_report_step_by_step() {
        let mut state = running_state();
        assert!(state.event(1, SpeedEvent::Ping(ping())).is_empty());
        state.event(1, SpeedEvent::Phase(Phase::Download));
        state.event(1, SpeedEvent::Progress(100.0));
        state.event(1, SpeedEvent::Progress(120.0));
        let Run::Running { phase, samples, .. } = &state.run else {
            panic!("expected a running test");
        };
        assert_eq!(*phase, Phase::Download);
        assert_eq!(samples, &[100.0, 120.0]);
        state.event(1, SpeedEvent::Result(result(Direction::Download)));
        state.event(1, SpeedEvent::Phase(Phase::Upload));
        let Run::Running { samples, .. } = &state.run else {
            panic!("expected a running test");
        };
        assert!(samples.is_empty(), "a new step starts a new graph");
    }

    #[test]
    fn finishing_saves_one_record_per_direction_with_the_ping() {
        let mut state = running_state();
        state.event(1, SpeedEvent::Ping(ping()));
        state.event(1, SpeedEvent::Result(result(Direction::Download)));
        state.event(1, SpeedEvent::Result(result(Direction::Upload)));
        let saved = state.event(1, SpeedEvent::Finished);
        assert_eq!(saved.len(), 2);
        for measurement in &saved {
            let Measurement::Throughput(record) = measurement else {
                panic!("expected throughput records");
            };
            assert_eq!(record.place, "LAN (x)");
            assert!(record.ping.is_some());
        }
        assert!(matches!(state.run, Run::Done(ref report) if report.error.is_none()));
        assert!(!state.running());
    }

    #[test]
    fn a_failure_after_one_step_keeps_and_saves_what_finished() {
        let mut state = running_state();
        state.event(1, SpeedEvent::Result(result(Direction::Download)));
        let saved = state.event(1, SpeedEvent::Failed("the link broke".to_owned()));
        assert_eq!(saved.len(), 1);
        let Run::Done(report) = &state.run else {
            panic!("expected a finished report");
        };
        assert_eq!(report.error.as_deref(), Some("the link broke"));
        assert_eq!(report.results.len(), 1);
    }

    #[test]
    fn the_server_naming_itself_updates_the_place() {
        let mut state = running_state();
        state.event(
            1,
            SpeedEvent::Place("Internet (M-Lab, Tokyo, JP)".to_owned()),
        );
        let Run::Running { report, .. } = &state.run else {
            panic!("expected a running test");
        };
        assert_eq!(report.place, "Internet (M-Lab, Tokyo, JP)");
    }

    #[test]
    fn a_failed_ping_does_not_stop_the_test() {
        let mut state = running_state();
        state.event(1, SpeedEvent::PingFailed("no route".to_owned()));
        assert!(state.running());
    }

    #[test]
    fn an_event_from_an_older_test_is_ignored() {
        let mut state = running_state();
        assert!(state.event(9, SpeedEvent::Finished).is_empty());
        assert!(state.running());
    }

    #[test]
    fn stopping_returns_only_the_steps_that_finished() {
        let mut state = running_state();
        state.event(1, SpeedEvent::Result(result(Direction::Download)));
        let saved = state.stop_test();
        assert_eq!(saved.len(), 1);
        assert!(matches!(state.run, Run::Idle));
        assert!(state.event(1, SpeedEvent::Finished).is_empty());
    }

    #[test]
    fn stopping_when_nothing_runs_changes_nothing() {
        let mut state = SpeedState::new(None);
        state.run = Run::Failed("x".to_owned());
        assert!(state.stop_test().is_empty());
        assert!(matches!(state.run, Run::Failed(_)));
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
    fn m_lab_s_own_country_list_replaces_the_known_one_and_a_failure_keeps_it() {
        let mut state = SpeedState::new(None);
        let known = state.countries.len();
        state.countries_found(Err("offline".to_owned()));
        assert_eq!(state.countries.len(), known);
        state.countries_found(Ok(vec!["JP".to_owned(), "EE".to_owned()]));
        assert_eq!(state.countries.len(), 3);
    }

    #[test]
    fn a_chosen_country_that_m_lab_no_longer_lists_goes_back_to_nearest() {
        let mut state = SpeedState::new(None);
        let japan = state
            .countries
            .iter()
            .find(|choice| choice.to_string() == "Japan (JP)")
            .cloned()
            .unwrap();
        state.location = japan;
        state.countries_found(Ok(vec!["BR".to_owned()]));
        assert_eq!(state.location, LocationChoice::nearest());
    }

    #[test]
    fn found_servers_are_fresh_only_for_a_short_while() {
        let mut state = SpeedState::new(None);
        assert!(!state.server_addresses_are_fresh());
        state.locate_generation = 1;
        state.servers_found(1, Ok(vec![server("a.example")]));
        assert!(state.server_addresses_are_fresh());
        state.servers_fetched_at = Instant::now().checked_sub(ndt7::SERVER_ADDRESS_LIFETIME);
        assert!(!state.server_addresses_are_fresh());
    }

    #[test]
    fn searching_again_while_a_search_runs_does_nothing() {
        let mut state = SpeedState::new(None);
        state.locating = true;
        let generation = state.locate_generation;
        let _ = state.search_again();
        assert_eq!(state.locate_generation, generation);
    }

    #[test]
    fn an_answer_for_an_older_search_is_ignored() {
        let mut state = SpeedState::new(None);
        state.locate_generation = 5;
        state.servers_found(4, Ok(vec![server("old.example")]));
        assert!(state.servers.is_empty());
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
    fn a_default_lan_test_pings_then_downloads_then_uploads_and_the_server_sees_both() {
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
            let mut test = Box::pin(test_stream(target, TestKind::default(), 2, 7));
            let mut steps = Vec::new();
            while let Some(message) = test.next().await {
                let Message::SpeedEvent(7, event) = message else {
                    panic!("unexpected message {message:?}");
                };
                match event {
                    SpeedEvent::Phase(phase) => steps.push(format!("{phase:?}")),
                    SpeedEvent::Ping(ping) => {
                        assert_eq!(ping.answered, ping.probes);
                        steps.push("ping done".to_owned());
                    }
                    SpeedEvent::Result(result) => {
                        assert!(result.mbps > 0.0);
                        steps.push(format!("{} done", result.direction));
                    }
                    SpeedEvent::Failed(message) => panic!("the test failed: {message}"),
                    SpeedEvent::Finished => steps.push("finished".to_owned()),
                    SpeedEvent::Progress(_) | SpeedEvent::Place(_) | SpeedEvent::PingFailed(_) => {}
                }
            }
            assert_eq!(
                steps,
                [
                    "Ping",
                    "ping done",
                    "Download",
                    "Download done",
                    "Upload",
                    "Upload done",
                    "finished"
                ]
            );
            let mut directions = Vec::new();
            while directions.len() < 2 {
                let Some(Message::SpeedServed(stream)) = server.next().await else {
                    panic!("the server should report the streams it served");
                };
                if !directions.contains(&stream.direction) {
                    directions.push(stream.direction);
                }
            }
        });
    }

    #[test]
    fn a_download_only_test_skips_the_upload() {
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
            let mut test = Box::pin(test_stream(target, TestKind::DownloadOnly, 1, 3));
            let mut results = Vec::new();
            while let Some(Message::SpeedEvent(_, event)) = test.next().await {
                if let SpeedEvent::Result(result) = event {
                    results.push(result.direction);
                }
            }
            assert_eq!(results, [Direction::Download]);
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
