//! The Traffic tab state: ask a router or switch over SNMP how much each interface carries.

use std::net::Ipv4Addr;
use std::time::Duration;

use iced::Task;
use iced::task::Handle;
use lanping_core::gateway::default_gateway;
use lanping_core::snmp::SnmpCommunity;
use lanping_core::snmp_interfaces::{Agent, Interface, Poll, RateTracker, SnmpError};

use crate::app::Message;

/// Time between two readings. Short enough that 32-bit counters, which wrap in about 34
/// seconds at 1 Gbit/s, are read before they wrap twice.
const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Readings in a row that may fail before the monitor gives up.
const MAX_FAILURES: u8 = 3;
const SECONDS_PER_MINUTE: u64 = 60;
const SECONDS_PER_HOUR: u64 = 3600;
const SECONDS_PER_DAY: u64 = 86_400;

pub enum Status {
    Idle,
    /// Listing the interfaces.
    Connecting,
    Running,
    Failed(String),
}

pub struct TrafficState {
    pub target: String,
    /// The gateway of this network, shown as a hint in the empty address field.
    pub gateway_hint: Option<Ipv4Addr>,
    pub show_all: bool,
    pub status: Status,
    pub interfaces: Vec<Interface>,
    pub tracker: RateTracker,
    /// Readings that failed since the last good one.
    pub failures: u8,
    agent: Option<Agent>,
    /// Counts starts and stops, so an answer from a stopped monitor is ignored.
    generation: u64,
    handle: Option<Handle>,
}

impl TrafficState {
    pub fn new() -> Self {
        Self {
            target: String::new(),
            gateway_hint: default_gateway(),
            show_all: false,
            status: Status::Idle,
            interfaces: Vec::new(),
            tracker: RateTracker::default(),
            failures: 0,
            agent: None,
            generation: 0,
            handle: None,
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.status, Status::Connecting | Status::Running)
    }

    /// Starts over against the typed device, using `community`.
    pub fn start(&mut self, community: &str) -> Task<Message> {
        let (ip, port) = match parse_target(&self.target) {
            Ok(target) => target,
            Err(message) => {
                self.status = Status::Failed(message);
                return Task::none();
            }
        };
        if community.trim().is_empty() {
            self.status = Status::Failed(
                "Enter the SNMP community of the device. LanPing never guesses it.".to_owned(),
            );
            return Task::none();
        }
        self.stop();
        self.generation += 1;
        self.status = Status::Connecting;
        self.interfaces.clear();
        self.tracker = RateTracker::default();
        self.failures = 0;
        let mut agent = Agent::new(ip, SnmpCommunity(community.trim().to_owned()));
        if let Some(port) = port {
            agent = agent.with_port(port);
        }
        self.agent = Some(agent.clone());
        let generation = self.generation;
        let (task, handle) = Task::perform(async move { agent.discover().await }, move |result| {
            Message::TrafficDiscovered(generation, result)
        })
        .abortable();
        self.handle = Some(handle);
        task
    }

    pub fn stop(&mut self) {
        self.generation += 1;
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        if self.running() {
            self.status = Status::Idle;
        }
    }

    pub fn discovered(
        &mut self,
        generation: u64,
        result: Result<Vec<Interface>, SnmpError>,
    ) -> Task<Message> {
        if generation != self.generation || !matches!(self.status, Status::Connecting) {
            return Task::none();
        }
        match result {
            Ok(interfaces) => {
                self.interfaces = interfaces;
                self.status = Status::Running;
                self.next_poll(Duration::ZERO)
            }
            Err(error) => {
                self.status = Status::Failed(error.to_string());
                self.handle = None;
                Task::none()
            }
        }
    }

    pub fn polled(&mut self, generation: u64, result: Result<Poll, SnmpError>) -> Task<Message> {
        if generation != self.generation || !matches!(self.status, Status::Running) {
            return Task::none();
        }
        match result {
            Ok(poll) => {
                self.failures = 0;
                for interface in &mut self.interfaces {
                    if let Some(reading) = poll.readings.get(&interface.index) {
                        interface.up = reading.up;
                    }
                }
                self.tracker.record(poll);
            }
            Err(error) => {
                self.failures += 1;
                if self.failures >= MAX_FAILURES {
                    self.status = Status::Failed(error.to_string());
                    self.handle = None;
                    return Task::none();
                }
            }
        }
        self.next_poll(POLL_INTERVAL)
    }

    fn next_poll(&mut self, delay: Duration) -> Task<Message> {
        let Some(agent) = self.agent.clone() else {
            return Task::none();
        };
        let interfaces = self.interfaces.clone();
        let generation = self.generation;
        let poll = async move {
            tokio::time::sleep(delay).await;
            agent.poll(&interfaces).await
        };
        let (task, handle) = Task::perform(poll, move |result| {
            Message::TrafficPolled(generation, result)
        })
        .abortable();
        self.handle = Some(handle);
        task
    }

    /// The interfaces to list: the ones that are up, unless all are asked for. Loopbacks only
    /// show with all.
    pub fn visible(&self) -> Vec<&Interface> {
        self.interfaces
            .iter()
            .filter(|interface| self.show_all || (interface.up && !interface.is_loopback()))
            .collect()
    }
}

/// `192.168.0.1`, or `192.168.0.1:1161` for a device that listens on another port than 161.
pub fn parse_target(text: &str) -> Result<(Ipv4Addr, Option<u16>), String> {
    let text = text.trim();
    let (address, port) = match text.split_once(':') {
        Some((address, port)) => (
            address,
            Some(
                port.parse::<u16>()
                    .ok()
                    .filter(|port| *port != 0)
                    .ok_or_else(|| "The port must be a number from 1 to 65535".to_owned())?,
            ),
        ),
        None => (text, None),
    };
    address
        .parse::<Ipv4Addr>()
        .map(|ip| (ip, port))
        .map_err(|_| {
            "Enter the IPv4 address of the router or switch, with :port if it is not 161".to_owned()
        })
}

/// `950 bit/s`, `12.3 kbit/s`, `4.5 Mbit/s`, `1.2 Gbit/s`.
pub fn format_rate(bits_per_second: f64) -> String {
    const UNITS: [(&str, f64); 3] = [("Gbit/s", 1e9), ("Mbit/s", 1e6), ("kbit/s", 1e3)];
    for (unit, size) in UNITS {
        if bits_per_second >= size {
            return format!("{:.1} {unit}", bits_per_second / size);
        }
    }
    format!("{bits_per_second:.0} bit/s")
}

/// `3d 4h`, `5h 12m` or `7m`.
pub fn format_uptime(seconds: u64) -> String {
    let (days, hours, minutes) = (
        seconds / SECONDS_PER_DAY,
        seconds % SECONDS_PER_DAY / SECONDS_PER_HOUR,
        seconds % SECONDS_PER_HOUR / SECONDS_PER_MINUTE,
    );
    match (days, hours) {
        (0, 0) => format!("{minutes}m"),
        (0, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h"),
    }
}

/// How full the link is: the busier direction against the link speed, as a percentage.
pub fn utilization_percent(busiest_bps: f64, speed_mbps: Option<u64>) -> Option<f64> {
    let speed_mbps = speed_mbps.filter(|speed| *speed > 0)?;
    Some(busiest_bps / (speed_mbps as f64 * 1e6) * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn interface(index: u32, up: bool, kind: u64) -> Interface {
        Interface {
            index,
            name: format!("if{index}"),
            alias: None,
            kind,
            speed_mbps: Some(1000),
            up,
            wide: true,
        }
    }

    fn running() -> TrafficState {
        let mut state = TrafficState::new();
        state.generation = 1;
        state.status = Status::Running;
        state
    }

    #[test]
    fn a_bad_address_or_a_missing_community_stops_before_asking() {
        let mut state = TrafficState::new();
        state.target = "router".to_owned();
        let _ = state.start("public");
        assert!(matches!(state.status, Status::Failed(ref m) if m.contains("IPv4")));
        state.target = "192.168.0.1".to_owned();
        let _ = state.start("  ");
        assert!(matches!(state.status, Status::Failed(ref m) if m.contains("community")));
        assert!(!state.running());
    }

    #[test]
    fn reads_an_address_with_and_without_a_port() {
        assert_eq!(
            parse_target(" 192.168.0.1 "),
            Ok(("192.168.0.1".parse().unwrap(), None))
        );
        assert_eq!(
            parse_target("127.0.0.1:1161"),
            Ok(("127.0.0.1".parse().unwrap(), Some(1161)))
        );
        for bad in [
            "",
            "router",
            "1.2.3.4:0",
            "1.2.3.4:x",
            "1.2.3.4:70000",
            ":161",
        ] {
            assert!(parse_target(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn by_default_only_interfaces_that_are_up_and_not_loopback_are_listed() {
        let mut state = running();
        state.interfaces = vec![
            interface(1, true, 24),
            interface(2, true, 6),
            interface(3, false, 6),
        ];
        let shown: Vec<u32> = state.visible().iter().map(|i| i.index).collect();
        assert_eq!(shown, [2]);
        state.show_all = true;
        assert_eq!(state.visible().len(), 3);
    }

    #[test]
    fn a_failed_listing_shows_why_and_stops() {
        let mut state = TrafficState::new();
        state.generation = 1;
        state.status = Status::Connecting;
        let _ = state.discovered(1, Err(SnmpError::NoAnswer));
        assert!(matches!(state.status, Status::Failed(ref m) if m.contains("community")));
    }

    #[test]
    fn an_answer_from_a_stopped_monitor_is_ignored() {
        let mut state = running();
        state.stop();
        let _ = state.polled(1, Err(SnmpError::NoAnswer));
        assert!(matches!(state.status, Status::Idle));
        assert_eq!(state.failures, 0);
    }

    #[test]
    fn it_gives_up_only_after_three_failed_readings_in_a_row() {
        let mut state = running();
        let _ = state.polled(1, Err(SnmpError::NoAnswer));
        let _ = state.polled(1, Err(SnmpError::NoAnswer));
        assert!(matches!(state.status, Status::Running));
        let _ = state.polled(1, Err(SnmpError::NoAnswer));
        assert!(matches!(state.status, Status::Failed(_)));
    }

    #[test]
    fn a_good_reading_resets_the_failure_count_and_updates_status() {
        let mut state = running();
        state.interfaces = vec![interface(2, true, 6)];
        let _ = state.polled(1, Err(SnmpError::NoAnswer));
        let poll = Poll {
            taken: std::time::Instant::now(),
            uptime_ticks: Some(100),
            readings: [(
                2,
                lanping_core::snmp_interfaces::Reading {
                    in_octets: 1,
                    out_octets: 1,
                    up: false,
                    wide: true,
                },
            )]
            .into(),
        };
        let _ = state.polled(1, Ok(poll));
        assert_eq!(state.failures, 0);
        assert!(!state.interfaces[0].up, "the interface went down");
    }

    #[test]
    fn rates_are_shown_in_the_unit_that_fits() {
        assert_eq!(format_rate(0.0), "0 bit/s");
        assert_eq!(format_rate(950.0), "950 bit/s");
        assert_eq!(format_rate(12_345.0), "12.3 kbit/s");
        assert_eq!(format_rate(4_500_000.0), "4.5 Mbit/s");
        assert_eq!(format_rate(1_200_000_000.0), "1.2 Gbit/s");
    }

    #[test]
    fn uptime_is_shown_in_the_two_largest_units() {
        assert_eq!(format_uptime(7 * 60), "7m");
        assert_eq!(format_uptime(5 * 3600 + 12 * 60), "5h 12m");
        assert_eq!(format_uptime(3 * 86_400 + 4 * 3600 + 59), "3d 4h");
    }

    #[test]
    fn utilization_needs_a_link_speed() {
        assert_eq!(utilization_percent(500e6, Some(1000)), Some(50.0));
        assert_eq!(utilization_percent(500e6, None), None);
        assert_eq!(utilization_percent(500e6, Some(0)), None);
    }
}
