//! Repeated traceroute with per-hop statistics, in the style of MTR.
//!
//! Each round runs the system's own traceroute once, so no raw-socket
//! privileges are needed. Statistics build up over the rounds.

use std::fmt;
use std::net::IpAddr;
use std::ops::Deref;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use crate::rtt_stats::RttStats;

/// Longest one round may run before it is stopped.
const ROUND_LIMIT: Duration = Duration::from_secs(40);
const MAX_HOPS: &str = "20";
const MAX_TARGET_LENGTH: usize = 253;

/// What one hop did in one round.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HopSample {
    /// Distance from the sender, starting at 1.
    pub hop: u8,
    /// The router that answered. `None` when nothing answered.
    pub address: Option<IpAddr>,
    pub rtt_ms: Option<f64>,
}

/// Why a round produced no hops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TraceError {
    InvalidTarget,
    ToolMissing,
    TimedOut,
    Failed(String),
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTarget => f.write_str("Enter a host name or an IP address"),
            Self::ToolMissing => f.write_str("Could not run traceroute. Is it installed?"),
            Self::TimedOut => f.write_str("The traceroute took too long"),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

/// Checks that `target` is a host name or IP address, and returns it trimmed.
/// A leading dash is refused so the traceroute tool cannot read it as an option.
pub fn validate_target(target: &str) -> Result<&str, TraceError> {
    let target = target.trim();
    let allowed = |c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_');
    let valid = !target.is_empty()
        && target.len() <= MAX_TARGET_LENGTH
        && !target.starts_with('-')
        && target.chars().all(allowed);
    if valid {
        Ok(target)
    } else {
        Err(TraceError::InvalidTarget)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Traceroute,
    Tracepath,
    Tracert,
}

impl Tool {
    fn program(self) -> &'static str {
        match self {
            Self::Traceroute => "traceroute",
            Self::Tracepath => "tracepath",
            Self::Tracert => "tracert",
        }
    }

    fn arguments(self, target: &str) -> Vec<String> {
        let flags: &[&str] = match self {
            Self::Traceroute => &["-n", "-q", "1", "-w", "1", "-m", MAX_HOPS],
            Self::Tracepath => &["-n", "-m", MAX_HOPS],
            Self::Tracert => &["-d", "-h", MAX_HOPS, "-w", "1000"],
        };
        flags
            .iter()
            .map(|flag| (*flag).to_owned())
            .chain(std::iter::once(target.to_owned()))
            .collect()
    }

    fn parse(self, text: &str) -> Vec<HopSample> {
        match self {
            Self::Traceroute => parse_traceroute(text),
            Self::Tracepath => parse_tracepath(text),
            Self::Tracert => parse_tracert(text),
        }
    }
}

/// Tools to try in order; the first one installed on this system is used.
fn tools() -> &'static [Tool] {
    if cfg!(windows) {
        &[Tool::Tracert]
    } else {
        &[Tool::Traceroute, Tool::Tracepath]
    }
}

/// Runs one traceroute round against `target` and returns what each hop did.
pub async fn run_round(target: &str) -> Result<Vec<HopSample>, TraceError> {
    let target = validate_target(target)?;
    for &tool in tools() {
        let run = Command::new(tool.program())
            .args(tool.arguments(target))
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output();
        let output = match timeout(ROUND_LIMIT, run).await {
            Err(_) => return Err(TraceError::TimedOut),
            Ok(Err(_)) => continue,
            Ok(Ok(output)) => output,
        };
        let samples = tool.parse(&String::from_utf8_lossy(&output.stdout));
        if !samples.is_empty() {
            return Ok(samples);
        }
        return Err(TraceError::Failed(failure_reason(tool, &output)));
    }
    Err(TraceError::ToolMissing)
}

fn failure_reason(tool: Tool, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    [stderr, stdout]
        .iter()
        .flat_map(|text| text.lines())
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(|| format!("{} failed", tool.program()), str::to_owned)
}

/// Parses `traceroute -n -q 1`: ` 1  192.168.0.1  2.138 ms` or ` 4  *`.
pub fn parse_traceroute(text: &str) -> Vec<HopSample> {
    text.lines()
        .filter_map(|line| {
            let mut tokens = line.split_whitespace();
            let hop: u8 = tokens.next()?.parse().ok()?;
            let first = tokens.next()?;
            if first == "*" {
                return Some(timeout_sample(hop));
            }
            let address: IpAddr = first.parse().ok()?;
            let rtt_ms = finite_milliseconds(tokens.next()?)?;
            Some(HopSample {
                hop,
                address: Some(address),
                rtt_ms: Some(rtt_ms),
            })
        })
        .collect()
}

/// Parses `tracepath -n`: ` 2:  172.16.69.1   2.661ms` or ` 3:  no reply`.
/// Each hop is printed twice at the start; the first line wins.
pub fn parse_tracepath(text: &str) -> Vec<HopSample> {
    let mut samples: Vec<HopSample> = Vec::new();
    for line in text.lines() {
        let mut tokens = line.split_whitespace();
        let Some(label) = tokens.next().and_then(|token| token.strip_suffix(':')) else {
            continue;
        };
        let Ok(hop) = label.parse::<u8>() else {
            continue;
        };
        if samples.iter().any(|sample| sample.hop == hop) {
            continue;
        }
        let rest: Vec<&str> = tokens.collect();
        let sample = match rest.first().and_then(|token| token.parse::<IpAddr>().ok()) {
            Some(address) => HopSample {
                hop,
                address: Some(address),
                rtt_ms: rest.iter().find_map(|token| parse_milliseconds(token)),
            },
            None if rest.starts_with(&["no", "reply"]) => timeout_sample(hop),
            None => continue,
        };
        samples.push(sample);
    }
    samples
}

/// Parses `tracert -d`: `  1    <1 ms    2 ms    1 ms  192.168.0.1` or
/// `  3     *        *        *     Request timed out.`.
/// Only English output is understood.
pub fn parse_tracert(text: &str) -> Vec<HopSample> {
    text.lines()
        .filter_map(|line| {
            let tokens: Vec<&str> = line.split_whitespace().collect();
            let hop: u8 = tokens.first()?.parse().ok()?;
            let address: Option<IpAddr> = tokens.last()?.parse().ok();
            let times: Vec<f64> = tokens
                .windows(2)
                .filter(|pair| pair[1] == "ms")
                .filter_map(|pair| finite_milliseconds(pair[0].trim_start_matches('<')))
                .collect();
            if times.is_empty() {
                let timed_out = tokens.iter().skip(1).any(|token| *token == "*");
                return timed_out.then(|| timeout_sample(hop));
            }
            Some(HopSample {
                hop,
                address,
                rtt_ms: Some(times.iter().sum::<f64>() / times.len() as f64),
            })
        })
        .collect()
}

fn timeout_sample(hop: u8) -> HopSample {
    HopSample {
        hop,
        address: None,
        rtt_ms: None,
    }
}

/// `2.661ms` as 2.661.
fn parse_milliseconds(token: &str) -> Option<f64> {
    finite_milliseconds(token.strip_suffix("ms")?)
}

/// A number of milliseconds. `NaN` and `inf`, which `f64` would also read, are refused so
/// one odd line cannot poison the statistics.
fn finite_milliseconds(text: &str) -> Option<f64> {
    text.parse::<f64>().ok().filter(|value| value.is_finite())
}

/// Everything learned about one hop over the rounds so far.
/// The round trip statistics are reachable directly: `hop.sent`, `hop.average_ms()`, and so on.
#[derive(Debug, Clone, PartialEq)]
pub struct HopStats {
    pub hop: u8,
    /// The router that answered most recently.
    pub address: Option<IpAddr>,
    rtt: RttStats,
}

impl Deref for HopStats {
    type Target = RttStats;

    fn deref(&self) -> &RttStats {
        &self.rtt
    }
}

impl HopStats {
    /// Round trips kept per hop for the graph.
    pub const HISTORY_LENGTH: usize = 40;

    fn new(hop: u8) -> Self {
        Self {
            hop,
            address: None,
            rtt: RttStats::new(Self::HISTORY_LENGTH),
        }
    }

    fn record(&mut self, sample: Option<&HopSample>) {
        if let Some(address) = sample.and_then(|sample| sample.address) {
            self.address = Some(address);
        }
        self.rtt.record(sample.and_then(|sample| sample.rtt_ms));
    }
}

/// Statistics for every hop on the path to one target.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TraceStats {
    hops: Vec<HopStats>,
    rounds: u32,
}

impl TraceStats {
    pub fn hops(&self) -> &[HopStats] {
        &self.hops
    }

    pub fn rounds(&self) -> u32 {
        self.rounds
    }

    /// Adds one round. A hop the round reached but that stayed silent counts as lost.
    /// Hops beyond the end of this round were not probed and are left alone.
    pub fn record_round(&mut self, samples: &[HopSample]) {
        let Some(last_hop) = samples.iter().map(|sample| sample.hop).max() else {
            return;
        };
        self.rounds += 1;
        while self.hops.len() < usize::from(last_hop) {
            let number = self.hops.len() as u8 + 1;
            self.hops.push(HopStats::new(number));
        }
        for stats in self.hops.iter_mut().filter(|stats| stats.hop <= last_hop) {
            let sample = samples.iter().find(|sample| sample.hop == stats.hop);
            stats.record(sample);
        }
    }
}

/// The statistics as tab-separated text with a header line.
pub fn stats_as_text(stats: &TraceStats) -> String {
    let header = "Hop\tAddress\tLoss\tSent\tLast\tAverage\tBest\tWorst\tJitter";
    let milliseconds =
        |value: Option<f64>| value.map_or_else(|| "-".to_owned(), |v| format!("{v:.1}"));
    let lines = stats.hops().iter().map(|hop| {
        [
            hop.hop.to_string(),
            hop.address
                .map_or_else(|| "???".to_owned(), |a| a.to_string()),
            format!("{:.0}%", hop.loss_percent()),
            hop.sent.to_string(),
            milliseconds(hop.last_ms),
            milliseconds(hop.average_ms()),
            milliseconds(hop.best_ms),
            milliseconds(hop.worst_ms),
            milliseconds(hop.jitter_ms()),
        ]
        .join("\t")
    });
    std::iter::once(header.to_owned())
        .chain(lines)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRACEROUTE: &str = "\
traceroute to 1.1.1.1 (1.1.1.1), 6 hops max, 60 byte packets
 1  192.168.0.1  2.138 ms
 2  172.16.69.1  3.282 ms
 3  *
 4  1.1.1.1  4.508 ms
";

    const TRACEPATH: &str = "\
 1?: [LOCALHOST]                      pmtu 1500
 1:  192.168.0.1                                           1.909ms
 1:  192.168.0.1                                           2.301ms
 2:  172.16.69.1                                           2.661ms
 3:  no reply
 4:  1.1.1.1                                               4.575ms reached
     Resume: pmtu 1500 hops 4 back 4
";

    const TRACERT: &str = "\
Tracing route to one.one.one.one [1.1.1.1]
over a maximum of 20 hops:

  1    <1 ms    <1 ms     1 ms  192.168.0.1
  2     3 ms     5 ms     4 ms  172.16.69.1
  3     *        *        *     Request timed out.
  4     5 ms     5 ms     6 ms  1.1.1.1

Trace complete.
";

    fn address(text: &str) -> Option<IpAddr> {
        Some(text.parse().unwrap())
    }

    fn sample(hop: u8, rtt_ms: Option<f64>) -> HopSample {
        HopSample {
            hop,
            address: rtt_ms.map(|_| "10.0.0.1".parse().unwrap()),
            rtt_ms,
        }
    }

    #[test]
    fn parses_traceroute_hops_and_timeouts() {
        let parsed = parse_traceroute(TRACEROUTE);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].address, address("192.168.0.1"));
        assert_eq!(parsed[0].rtt_ms, Some(2.138));
        assert_eq!(parsed[2], timeout_sample(3));
        assert_eq!(parsed[3].hop, 4);
    }

    #[test]
    fn parses_tracepath_and_keeps_the_first_line_of_a_repeated_hop() {
        let parsed = parse_tracepath(TRACEPATH);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].rtt_ms, Some(1.909));
        assert_eq!(parsed[2], timeout_sample(3));
        assert_eq!(parsed[3].address, address("1.1.1.1"));
    }

    #[test]
    fn parses_tracert_and_averages_the_probes() {
        let parsed = parse_tracert(TRACERT);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].address, address("192.168.0.1"));
        assert_eq!(parsed[0].rtt_ms, Some(1.0));
        assert_eq!(parsed[1].rtt_ms, Some(4.0));
        assert_eq!(parsed[2], timeout_sample(3));
        assert_eq!(parsed[3].rtt_ms, Some(16.0 / 3.0));
    }

    #[test]
    fn numbers_that_are_not_finite_are_not_round_trip_times() {
        assert!(parse_traceroute(" 1  10.0.0.1  NaN ms").is_empty());
        assert!(parse_traceroute(" 1  10.0.0.1  inf ms").is_empty());
        let tracepath = parse_tracepath(" 1:  10.0.0.1  infms");
        assert_eq!(tracepath.len(), 1);
        assert_eq!(tracepath[0].rtt_ms, None);
    }

    #[test]
    fn unrelated_output_has_no_hops() {
        assert!(parse_traceroute("nonexistent.invalid: Name or service not known").is_empty());
        assert!(parse_tracepath("").is_empty());
        assert!(parse_tracert("Unable to resolve target system name x.").is_empty());
    }

    #[test]
    fn accepts_host_names_and_addresses() {
        assert_eq!(validate_target(" example.com "), Ok("example.com"));
        assert_eq!(validate_target("192.168.0.1"), Ok("192.168.0.1"));
        assert_eq!(validate_target("fe80::1"), Ok("fe80::1"));
    }

    #[test]
    fn refuses_options_and_odd_characters() {
        for target in ["", "   ", "-g 1.2.3.4", "a b", "x;ls", "a/b"] {
            assert_eq!(
                validate_target(target),
                Err(TraceError::InvalidTarget),
                "{target}"
            );
        }
        let too_long = "a".repeat(MAX_TARGET_LENGTH + 1);
        assert_eq!(validate_target(&too_long), Err(TraceError::InvalidTarget));
    }

    #[test]
    fn statistics_follow_the_rounds() {
        let mut stats = TraceStats::default();
        stats.record_round(&[sample(1, Some(2.0)), sample(2, Some(10.0))]);
        stats.record_round(&[sample(1, Some(4.0)), sample(2, None)]);
        stats.record_round(&[sample(1, Some(3.0)), sample(2, Some(30.0))]);
        assert_eq!(stats.rounds(), 3);
        let first = &stats.hops()[0];
        assert_eq!(first.sent, 3);
        assert_eq!(first.average_ms(), Some(3.0));
        assert_eq!(first.best_ms, Some(2.0));
        assert_eq!(first.worst_ms, Some(4.0));
        assert_eq!(first.jitter_ms(), Some(1.5));
        let second = &stats.hops()[1];
        assert_eq!(second.received, 2);
        assert!((second.loss_percent() - 100.0 / 3.0).abs() < 1e-9);
        assert_eq!(second.last_ms, Some(30.0));
        assert_eq!(second.history(), [Some(10.0), None, Some(30.0)]);
    }

    #[test]
    fn a_silent_hop_inside_the_path_counts_as_lost() {
        let mut stats = TraceStats::default();
        stats.record_round(&[sample(1, Some(1.0)), sample(3, Some(9.0))]);
        assert_eq!(stats.hops().len(), 3);
        assert_eq!(stats.hops()[1].sent, 1);
        assert_eq!(stats.hops()[1].received, 0);
        assert_eq!(stats.hops()[1].loss_percent(), 100.0);
    }

    #[test]
    fn hops_past_a_shorter_round_are_not_counted() {
        let mut stats = TraceStats::default();
        stats.record_round(&[sample(1, Some(1.0)), sample(2, Some(2.0))]);
        stats.record_round(&[sample(1, Some(1.0))]);
        assert_eq!(stats.hops()[0].sent, 2);
        assert_eq!(stats.hops()[1].sent, 1);
    }

    #[test]
    fn an_empty_round_changes_nothing() {
        let mut stats = TraceStats::default();
        stats.record_round(&[]);
        assert_eq!(stats, TraceStats::default());
    }

    #[test]
    fn history_keeps_only_the_latest_round_trips() {
        let mut stats = TraceStats::default();
        for _ in 0..HopStats::HISTORY_LENGTH + 3 {
            stats.record_round(&[sample(1, Some(1.0))]);
        }
        assert_eq!(stats.hops()[0].history().len(), HopStats::HISTORY_LENGTH);
    }

    #[test]
    fn text_form_has_a_header_and_one_line_per_hop() {
        let mut stats = TraceStats::default();
        stats.record_round(&[sample(1, Some(2.0)), sample(2, None)]);
        let text = stats_as_text(&stats);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1], "1\t10.0.0.1\t0%\t1\t2.0\t2.0\t2.0\t2.0\t-");
        assert_eq!(lines[2], "2\t???\t100%\t1\t-\t-\t-\t-\t-");
    }
}
