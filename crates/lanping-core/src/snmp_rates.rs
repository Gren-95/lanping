//! Turns SNMP counter readings into rates and keeps the recent ones for a graph.

use std::collections::{HashMap, VecDeque};

use crate::snmp_interfaces::Poll;

/// Rates kept per interface for a graph.
pub const HISTORY_LENGTH: usize = 40;
const BITS_PER_BYTE: f64 = 8.0;
const COUNTER32_RANGE: u64 = 1 << 32;
const TICKS_PER_SECOND: u64 = 100;

/// Bits per second in and out of one interface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rate {
    pub in_bps: f64,
    pub out_bps: f64,
}

impl Rate {
    pub fn total_bps(self) -> f64 {
        self.in_bps + self.out_bps
    }
}

/// How much a counter grew. 32-bit counters wrap around, so a smaller number means a wrap.
/// A 64-bit counter never wraps in practice, so a smaller number means it was reset.
pub(crate) fn counter_growth(before: u64, now: u64, wide: bool) -> Option<u64> {
    if wide {
        now.checked_sub(before)
    } else {
        Some(now.wrapping_sub(before) & (COUNTER32_RANGE - 1))
    }
}

/// Turns readings into rates and keeps recent ones for a graph.
#[derive(Debug, Default)]
pub struct RateTracker {
    previous: Option<Poll>,
    latest: HashMap<u32, Rate>,
    history: HashMap<u32, VecDeque<Rate>>,
}

impl RateTracker {
    /// Adds a reading. The first one gives no rates; each later one gives the rates since the
    /// one before. A device that restarted, or an interface whose counter was reset, gives
    /// none for that step.
    pub fn record(&mut self, poll: Poll) {
        let restarted = match (&self.previous, poll.uptime_ticks) {
            (Some(previous), Some(now)) => previous.uptime_ticks.is_some_and(|before| now < before),
            _ => false,
        };
        self.latest.clear();
        if let Some(previous) = self.previous.as_ref().filter(|_| !restarted) {
            let seconds = poll.taken.duration_since(previous.taken).as_secs_f64();
            if seconds > 0.0 {
                for (index, now) in &poll.readings {
                    let Some(before) = previous.readings.get(index) else {
                        continue;
                    };
                    let (Some(in_growth), Some(out_growth)) = (
                        counter_growth(before.in_octets, now.in_octets, now.wide),
                        counter_growth(before.out_octets, now.out_octets, now.wide),
                    ) else {
                        continue;
                    };
                    let rate = Rate {
                        in_bps: in_growth as f64 * BITS_PER_BYTE / seconds,
                        out_bps: out_growth as f64 * BITS_PER_BYTE / seconds,
                    };
                    self.latest.insert(*index, rate);
                    let history = self.history.entry(*index).or_default();
                    history.push_back(rate);
                    if history.len() > HISTORY_LENGTH {
                        history.pop_front();
                    }
                }
            }
        }
        self.previous = Some(poll);
    }

    /// The rate over the last step, when there was one.
    pub fn rate(&self, index: u32) -> Option<Rate> {
        self.latest.get(&index).copied()
    }

    /// Recent rates, oldest first.
    pub fn history(&self, index: u32) -> Vec<Rate> {
        self.history
            .get(&index)
            .map(|rates| rates.iter().copied().collect())
            .unwrap_or_default()
    }

    /// The device's uptime in seconds at the last reading.
    pub fn uptime_seconds(&self) -> Option<u64> {
        self.previous
            .as_ref()
            .and_then(|poll| poll.uptime_ticks)
            .map(|ticks| ticks / TICKS_PER_SECOND)
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::snmp_interfaces::Reading;

    #[test]
    fn counters_grow_with_wraps_for_32_bit_and_resets_for_64_bit() {
        assert_eq!(counter_growth(100, 250, false), Some(150));
        assert_eq!(counter_growth(COUNTER32_RANGE - 10, 20, false), Some(30));
        assert_eq!(counter_growth(100, 250, true), Some(150));
        assert_eq!(counter_growth(250, 100, true), None);
    }

    fn poll_at(taken: Instant, ticks: u64, readings: &[(u32, Reading)]) -> Poll {
        Poll {
            taken,
            uptime_ticks: Some(ticks),
            readings: readings.iter().copied().collect(),
        }
    }

    fn reading(in_octets: u64, out_octets: u64, wide: bool) -> Reading {
        Reading {
            in_octets,
            out_octets,
            up: true,
            wide,
        }
    }

    #[test]
    fn rates_come_from_two_readings() {
        let start = Instant::now();
        let mut tracker = RateTracker::default();
        tracker.record(poll_at(start, 1000, &[(2, reading(0, 0, true))]));
        assert_eq!(tracker.rate(2), None, "one reading gives no rate");
        // 25 MB in and 12.5 MB out over 2 seconds.
        tracker.record(poll_at(
            start + Duration::from_secs(2),
            1200,
            &[(2, reading(25_000_000, 12_500_000, true))],
        ));
        let rate = tracker.rate(2).unwrap();
        assert_eq!(rate.in_bps, 100_000_000.0);
        assert_eq!(rate.out_bps, 50_000_000.0);
        assert_eq!(rate.total_bps(), 150_000_000.0);
        assert_eq!(tracker.history(2).len(), 1);
        assert_eq!(tracker.uptime_seconds(), Some(12));
    }

    #[test]
    fn a_32_bit_wrap_is_counted_and_a_restart_gives_no_rate() {
        let start = Instant::now();
        let mut tracker = RateTracker::default();
        let near_wrap = COUNTER32_RANGE - 1000;
        tracker.record(poll_at(start, 1000, &[(3, reading(near_wrap, 0, false))]));
        tracker.record(poll_at(
            start + Duration::from_secs(1),
            1100,
            &[(3, reading(3000, 0, false))],
        ));
        assert_eq!(tracker.rate(3).unwrap().in_bps, 4000.0 * 8.0);
        // The device restarted: its uptime dropped and its counters began again.
        tracker.record(poll_at(
            start + Duration::from_secs(2),
            50,
            &[(3, reading(10, 0, false))],
        ));
        assert_eq!(tracker.rate(3), None);
        tracker.record(poll_at(
            start + Duration::from_secs(3),
            150,
            &[(3, reading(1010, 0, false))],
        ));
        assert_eq!(tracker.rate(3).unwrap().in_bps, 8000.0);
    }

    #[test]
    fn a_reset_64_bit_counter_is_skipped_and_history_is_bounded() {
        let start = Instant::now();
        let mut tracker = RateTracker::default();
        tracker.record(poll_at(start, 100, &[(2, reading(5000, 5000, true))]));
        tracker.record(poll_at(
            start + Duration::from_secs(1),
            200,
            &[(2, reading(10, 10, true))],
        ));
        assert_eq!(tracker.rate(2), None);
        let mut octets = 0;
        for step in 2..HISTORY_LENGTH as u64 + 10 {
            octets += 1000;
            tracker.record(poll_at(
                start + Duration::from_secs(step),
                200 + step,
                &[(2, reading(octets, octets, true))],
            ));
        }
        assert_eq!(tracker.history(2).len(), HISTORY_LENGTH);
    }
}
