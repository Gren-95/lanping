//! Saved latency runs and throughput tests, one JSON file each, next to the saved scans.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::files::{json_files, write_new_json};
use crate::latency::LatencySeries;
use crate::ping::PingSummary;
use crate::throughput::ThroughputResult;

/// Files listed at most, newest first.
pub const LIST_LIMIT: usize = 100;

/// A latency run that was stopped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LatencyRecord {
    pub target: String,
    /// `ICMP ping` or `TCP connect`.
    pub method: String,
    pub finished_at_unix: u64,
    pub probes: u32,
    pub loss_percent: f64,
    pub average_ms: Option<f64>,
    pub best_ms: Option<f64>,
    pub worst_ms: Option<f64>,
    pub jitter_ms: Option<f64>,
    /// The last probes, oldest first, for the graph. `None` is a probe with no answer.
    pub recent_ms: Vec<Option<f64>>,
}

impl LatencyRecord {
    pub fn new(target: &str, method: &str, finished_at_unix: u64, series: &LatencySeries) -> Self {
        Self {
            target: target.trim().to_owned(),
            method: method.to_owned(),
            finished_at_unix,
            probes: series.sent,
            loss_percent: series.loss_percent(),
            average_ms: series.average_ms(),
            best_ms: series.best_ms,
            worst_ms: series.worst_ms,
            jitter_ms: series.jitter_ms(),
            recent_ms: series.history(),
        }
    }
}

/// One throughput test and where it was run against.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThroughputRecord {
    /// `Internet (speed.cloudflare.com)` or `LAN (192.168.0.5:5252)`.
    pub place: String,
    pub finished_at_unix: u64,
    pub result: ThroughputResult,
    /// How far away the server is, measured just before the test. Older files have none.
    #[serde(default)]
    pub ping: Option<PingSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum Measurement {
    Latency(LatencyRecord),
    Throughput(ThroughputRecord),
}

impl Measurement {
    pub fn finished_at_unix(&self) -> u64 {
        match self {
            Self::Latency(record) => record.finished_at_unix,
            Self::Throughput(record) => record.finished_at_unix,
        }
    }

    /// What the record is, for a list: `Latency to 1.1.1.1`, `Download, Internet (…)`.
    pub fn title(&self) -> String {
        match self {
            Self::Latency(record) => format!("Latency to {}", record.target),
            Self::Throughput(record) => {
                format!("{}, {}", record.result.direction, record.place)
            }
        }
    }

    /// The main numbers, in one line.
    pub fn summary(&self) -> String {
        match self {
            Self::Latency(record) => {
                let average = record.average_ms.map_or_else(
                    || "no answers".to_owned(),
                    |ms| format!("average {ms:.1} ms"),
                );
                format!(
                    "{average}, {:.1}% loss, {} probes",
                    record.loss_percent, record.probes
                )
            }
            Self::Throughput(record) => {
                let speed = format!(
                    "{:.1} Mbit/s over {:.0} s",
                    record.result.mbps, record.result.seconds
                );
                match &record.ping {
                    Some(ping) => format!("{speed}, ping {}", ping.describe()),
                    None => speed,
                }
            }
        }
    }

    fn file_stem(&self) -> String {
        let kind = match self {
            Self::Latency(_) => "latency",
            Self::Throughput(_) => "throughput",
        };
        format!("{kind}-{}", self.finished_at_unix())
    }
}

/// Writes the record into `directory` and returns the file path. A record finished in the
/// same second as another of its kind gets a number added, so nothing is overwritten.
pub fn save(directory: &Path, measurement: &Measurement) -> io::Result<PathBuf> {
    let json = serde_json::to_string(measurement).map_err(io::Error::other)?;
    write_new_json(directory, &measurement.file_stem(), &json)
}

/// Lists readable records, newest first, at most [`LIST_LIMIT`]. Corrupt files are skipped.
pub fn list(directory: &Path) -> Vec<(PathBuf, Measurement)> {
    let mut records: Vec<(PathBuf, Measurement)> = json_files(directory)
        .into_iter()
        .filter_map(|path| {
            let json = fs::read_to_string(&path).ok()?;
            let measurement = serde_json::from_str(&json).ok()?;
            Some((path, measurement))
        })
        .collect();
    records.sort_by_key(|(_, measurement)| std::cmp::Reverse(measurement.finished_at_unix()));
    records.truncate(LIST_LIMIT);
    records
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::throughput::Direction;

    fn scratch_directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "lanping-measurements-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&directory);
        directory
    }

    fn throughput(finished_at_unix: u64, mbps_bytes: u64) -> Measurement {
        Measurement::Throughput(ThroughputRecord {
            place: "LAN (10.0.0.2:5252)".to_owned(),
            finished_at_unix,
            result: ThroughputResult::new(Direction::Download, mbps_bytes, 1.0, vec![1.0]),
            ping: None,
        })
    }

    fn latency(finished_at_unix: u64) -> Measurement {
        let mut series = LatencySeries::default();
        series.record(1, Some(4.0));
        series.record(2, None);
        Measurement::Latency(LatencyRecord::new(
            " 1.1.1.1 ",
            "ICMP ping",
            finished_at_unix,
            &series,
        ))
    }

    #[test]
    fn saves_and_lists_both_kinds_newest_first() {
        let directory = scratch_directory("both");
        save(&directory, &throughput(100, 1_000_000)).unwrap();
        save(&directory, &latency(300)).unwrap();
        save(&directory, &throughput(200, 2_000_000)).unwrap();
        let listed = list(&directory);
        let times: Vec<u64> = listed.iter().map(|(_, m)| m.finished_at_unix()).collect();
        assert_eq!(times, [300, 200, 100]);
        assert!(matches!(listed[0].1, Measurement::Latency(_)));
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_second_record_in_the_same_second_does_not_overwrite_the_first() {
        let directory = scratch_directory("same-second");
        let first = save(&directory, &throughput(100, 1)).unwrap();
        let second = save(&directory, &throughput(100, 2)).unwrap();
        assert_ne!(first, second);
        assert_eq!(list(&directory).len(), 2);
        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn a_saved_latency_run_keeps_its_numbers() {
        let Measurement::Latency(record) = latency(5) else {
            panic!("expected a latency record");
        };
        assert_eq!(record.target, "1.1.1.1");
        assert_eq!(record.probes, 2);
        assert_eq!(record.loss_percent, 50.0);
        assert_eq!(record.average_ms, Some(4.0));
        assert_eq!(record.recent_ms, [Some(4.0), None]);
    }

    #[test]
    fn a_record_saved_before_ping_existed_still_loads() {
        let old = r#"{"kind":"Throughput","place":"LAN (x)","finished_at_unix":5,
            "result":{"direction":"Download","bytes":1,"seconds":1.0,"mbps":0.0,"samples_mbps":[]}}"#;
        let Measurement::Throughput(record) = serde_json::from_str(old).unwrap() else {
            panic!("expected a throughput record");
        };
        assert!(record.ping.is_none());
    }

    #[test]
    fn corrupt_and_foreign_files_are_skipped() {
        let directory = scratch_directory("corrupt");
        save(&directory, &latency(10)).unwrap();
        fs::write(directory.join("broken.json"), "{not json").unwrap();
        fs::write(directory.join("notes.txt"), "hello").unwrap();
        assert_eq!(list(&directory).len(), 1);
        fs::remove_dir_all(&directory).unwrap();
        assert!(list(&directory).is_empty());
    }

    #[test]
    fn describes_a_record_in_words() {
        assert_eq!(latency(1).title(), "Latency to 1.1.1.1");
        assert_eq!(latency(1).summary(), "average 4.0 ms, 50.0% loss, 2 probes");
        let test = throughput(1, 125_000_000);
        assert_eq!(test.title(), "Download, LAN (10.0.0.2:5252)");
        assert_eq!(test.summary(), "1000.0 Mbit/s over 1 s");
        let Measurement::Throughput(mut record) = test else {
            panic!("expected a throughput record");
        };
        let mut stats = crate::rtt_stats::RttStats::new(3);
        stats.record(Some(12.0));
        record.ping = Some(PingSummary::from_stats(&stats));
        assert_eq!(
            Measurement::Throughput(record).summary(),
            "1000.0 Mbit/s over 1 s, ping 12.0 ms"
        );
    }
}
