//! The values of the Speed tab: which test, how long, and what it reports.

use std::net::IpAddr;
use std::time::Instant;

use lanping_core::measurements::{Measurement, ThroughputRecord};
use lanping_core::ping::PingSummary;
use lanping_core::throughput::{Direction, ThroughputResult};

use crate::timestamp::unix_now;

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

/// The step of a test that is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Ping,
    Download,
    Upload,
}

impl std::fmt::Display for Phase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Ping => "Measuring ping",
            Self::Download => "Testing download",
            Self::Upload => "Testing upload",
        })
    }
}

/// What a run measures after the ping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TestKind {
    #[default]
    Both,
    DownloadOnly,
    UploadOnly,
}

impl TestKind {
    pub const ALL: [Self; 3] = [Self::Both, Self::DownloadOnly, Self::UploadOnly];

    pub(crate) fn directions(self) -> &'static [Direction] {
        match self {
            Self::Both => &[Direction::Download, Direction::Upload],
            Self::DownloadOnly => &[Direction::Download],
            Self::UploadOnly => &[Direction::Upload],
        }
    }
}

impl std::fmt::Display for TestKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Both => "Download and upload",
            Self::DownloadOnly => "Download only",
            Self::UploadOnly => "Upload only",
        })
    }
}

/// What a running test tells the screen, in order.
#[derive(Debug, Clone)]
pub enum SpeedEvent {
    /// The server in use, once it is known.
    Place(String),
    Phase(Phase),
    /// Megabits per second of each finished second of the current step.
    Progress(f64),
    Ping(PingSummary),
    PingFailed(String),
    Result(ThroughputResult),
    /// The test stopped early, with the reason.
    Failed(String),
    Finished,
}

/// Everything a test found so far.
pub struct Report {
    pub place: String,
    pub ping: Option<PingSummary>,
    pub ping_error: Option<String>,
    pub results: Vec<ThroughputResult>,
    /// Why the test stopped early.
    pub error: Option<String>,
}

impl Report {
    pub(crate) fn new(place: String) -> Self {
        Self {
            place,
            ping: None,
            ping_error: None,
            results: Vec::new(),
            error: None,
        }
    }

    /// One record per finished step, for the history.
    pub(crate) fn records(&self) -> Vec<Measurement> {
        let finished_at_unix = unix_now();
        self.results
            .iter()
            .map(|result| {
                Measurement::Throughput(ThroughputRecord {
                    place: self.place.clone(),
                    finished_at_unix,
                    result: result.clone(),
                    ping: self.ping.clone(),
                })
            })
            .collect()
    }
}

pub enum Run {
    Idle,
    Running {
        phase: Phase,
        /// Megabits per second of each finished second of the current step.
        samples: Vec<f64>,
        report: Report,
    },
    Done(Report),
    /// The test could not start.
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
    pub(crate) last_seen: Instant,
}
