//! Running statistics of round trip times, shared by the trace and latency tools.

use std::collections::VecDeque;

/// What a series of probes has shown so far. A probe with no answer is a loss.
#[derive(Debug, Clone, PartialEq)]
pub struct RttStats {
    pub sent: u32,
    pub received: u32,
    pub last_ms: Option<f64>,
    pub best_ms: Option<f64>,
    pub worst_ms: Option<f64>,
    total_ms: f64,
    previous_ms: Option<f64>,
    change_total_ms: f64,
    changes: u32,
    history: VecDeque<Option<f64>>,
    history_length: usize,
}

impl RttStats {
    /// Statistics that keep the latest `history_length` probes for a graph.
    pub fn new(history_length: usize) -> Self {
        Self {
            sent: 0,
            received: 0,
            last_ms: None,
            best_ms: None,
            worst_ms: None,
            total_ms: 0.0,
            previous_ms: None,
            change_total_ms: 0.0,
            changes: 0,
            history: VecDeque::with_capacity(history_length),
            history_length,
        }
    }

    /// Adds one probe: its round trip in milliseconds, or `None` when nothing answered.
    pub fn record(&mut self, rtt_ms: Option<f64>) {
        self.sent += 1;
        self.last_ms = rtt_ms;
        if let Some(rtt) = rtt_ms {
            self.received += 1;
            self.total_ms += rtt;
            self.best_ms = Some(self.best_ms.map_or(rtt, |best| best.min(rtt)));
            self.worst_ms = Some(self.worst_ms.map_or(rtt, |worst| worst.max(rtt)));
            if let Some(previous) = self.previous_ms {
                self.change_total_ms += (rtt - previous).abs();
                self.changes += 1;
            }
            self.previous_ms = Some(rtt);
        }
        self.history.push_back(rtt_ms);
        if self.history.len() > self.history_length {
            self.history.pop_front();
        }
    }

    /// Share of probes that got no answer, from 0 to 100.
    pub fn loss_percent(&self) -> f64 {
        if self.sent == 0 {
            return 0.0;
        }
        f64::from(self.sent - self.received) * 100.0 / f64::from(self.sent)
    }

    pub fn average_ms(&self) -> Option<f64> {
        (self.received > 0).then(|| self.total_ms / f64::from(self.received))
    }

    /// Mean change between one answer and the next.
    pub fn jitter_ms(&self) -> Option<f64> {
        (self.changes > 0).then(|| self.change_total_ms / f64::from(self.changes))
    }

    /// Recent probes, oldest first. `None` is a probe with no answer.
    pub fn history(&self) -> Vec<Option<f64>> {
        self.history.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_follow_the_probes() {
        let mut stats = RttStats::new(10);
        for rtt in [Some(2.0), None, Some(4.0), Some(3.0)] {
            stats.record(rtt);
        }
        assert_eq!((stats.sent, stats.received), (4, 3));
        assert_eq!(stats.loss_percent(), 25.0);
        assert_eq!(stats.average_ms(), Some(3.0));
        assert_eq!(stats.best_ms, Some(2.0));
        assert_eq!(stats.worst_ms, Some(4.0));
        assert_eq!(stats.last_ms, Some(3.0));
        assert_eq!(stats.jitter_ms(), Some(1.5));
        assert_eq!(stats.history(), [Some(2.0), None, Some(4.0), Some(3.0)]);
    }

    #[test]
    fn nothing_is_known_before_the_first_probe() {
        let stats = RttStats::new(3);
        assert_eq!(stats.loss_percent(), 0.0);
        assert_eq!(stats.average_ms(), None);
        assert_eq!(stats.jitter_ms(), None);
        assert!(stats.history().is_empty());
    }

    #[test]
    fn only_the_latest_probes_are_kept() {
        let mut stats = RttStats::new(2);
        for rtt in [1.0, 2.0, 3.0] {
            stats.record(Some(rtt));
        }
        assert_eq!(stats.history(), [Some(2.0), Some(3.0)]);
        assert_eq!(stats.sent, 3);
    }
}
