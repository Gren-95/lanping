//! The Latency tab: probe one host on an interval and keep a graph and a log.

use std::fmt;
use std::time::Duration;

use iced::widget::{
    Row as LayoutRow, button, column, container, pick_list, space, text, text_input,
};
use iced::{Alignment, Element, Length, Task};
use lanping_core::latency::{GRAPH_LENGTH, LatencyError, LatencySeries, Prober};
use lanping_core::measurements::{LatencyRecord, Measurement};
use lanping_core::trace::validate_target;

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, icon_button, lucide, with_tooltip};
use crate::job::Job;
use crate::results::pill;
use crate::sparkline::{Bar, chart};
use crate::theme::{
    self, BAD, BOLD, Edge, MIN_GRAPH_SCALE_MS, MUTED, NO_VALUE, SECTION_SPACING, WARN,
    latency_color, loss_color,
};
use crate::timestamp::unix_now;
use crate::ui::empty_state;
use crate::view::input_group;

const CHART_HEIGHT: f32 = 120.0;
const TILE_PADDING: f32 = 12.0;
const TILE_WIDTH: f32 = 120.0;

/// Time between two probes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Interval {
    #[default]
    Second,
    FiveSeconds,
    ThirtySeconds,
}

impl Interval {
    pub const ALL: [Self; 3] = [Self::Second, Self::FiveSeconds, Self::ThirtySeconds];

    fn duration(self) -> Duration {
        Duration::from_secs(match self {
            Self::Second => 1,
            Self::FiveSeconds => 5,
            Self::ThirtySeconds => 30,
        })
    }
}

impl fmt::Display for Interval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Second => "Every second",
            Self::FiveSeconds => "Every 5 seconds",
            Self::ThirtySeconds => "Every 30 seconds",
        })
    }
}

/// What the running or finished measurement knows.
#[derive(Default)]
pub struct LatencyState {
    target: String,
    interval: Interval,
    series: LatencySeries,
    prober: Option<Prober>,
    running: bool,
    error: Option<LatencyError>,
    job: Job,
}

impl LatencyState {
    pub fn set_target(&mut self, target: String) {
        self.target = target;
    }

    pub fn set_interval(&mut self, interval: Interval) {
        self.interval = interval;
    }

    /// Starts over against the typed target.
    pub fn start(&mut self) -> Task<Message> {
        if validate_target(&self.target).is_err() {
            self.error = Some(LatencyError::InvalidTarget);
            return Task::none();
        }
        self.stop();
        self.series = LatencySeries::default();
        self.prober = None;
        self.error = None;
        self.running = true;
        let target = self.target.clone();
        self.job.spawn(
            async move { Prober::connect(&target).await },
            Message::LatencyReady,
        )
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.job.cancel();
    }

    /// The target resolved: remember how to probe it and send the first probe.
    pub fn ready(
        &mut self,
        generation: u64,
        result: Result<Prober, LatencyError>,
    ) -> Task<Message> {
        if !self.job.is_current(generation) || !self.running {
            return Task::none();
        }
        match result {
            Ok(prober) => {
                self.prober = Some(prober);
                self.next_probe(Duration::ZERO)
            }
            Err(error) => {
                self.error = Some(error);
                self.running = false;
                self.job.finished();
                Task::none()
            }
        }
    }

    /// Records a probe and schedules the next one.
    pub fn sampled(&mut self, generation: u64, rtt_ms: Option<f64>) -> Task<Message> {
        if !self.job.is_current(generation) || !self.running {
            return Task::none();
        }
        self.series.record(unix_now(), rtt_ms);
        self.next_probe(self.interval.duration())
    }

    fn next_probe(&mut self, delay: Duration) -> Task<Message> {
        let Some(prober) = self.prober.clone() else {
            return Task::none();
        };
        let probe = async move {
            tokio::time::sleep(delay).await;
            prober.sample().await
        };
        self.job.spawn_next(probe, Message::LatencySample)
    }

    /// The run so far as a record for the history. `None` before the first probe.
    pub fn record_for_history(&self) -> Option<Measurement> {
        if self.series.sent == 0 {
            return None;
        }
        let method = self
            .prober
            .as_ref()
            .map_or("unknown", |prober| prober.method());
        Some(Measurement::Latency(LatencyRecord::new(
            &self.target,
            method,
            unix_now(),
            &self.series,
        )))
    }

    pub fn has_log(&self) -> bool {
        self.series.logged() > 0
    }

    pub fn log_as_csv(&self) -> String {
        self.series.log_as_csv()
    }

    /// A one-line summary for the clipboard.
    pub fn summary(&self) -> String {
        let value =
            |ms: Option<f64>| ms.map_or_else(|| NO_VALUE.to_owned(), |ms| format!("{ms:.1} ms"));
        let series = &self.series;
        format!(
            "{}: {} probes, {:.1}% loss, last {}, average {}, best {}, worst {}, jitter {}",
            self.target.trim(),
            series.sent,
            series.loss_percent(),
            value(series.last_ms),
            value(series.average_ms()),
            value(series.best_ms),
            value(series.worst_ms),
            value(series.jitter_ms()),
        )
    }

    pub fn default_log_name(&self) -> String {
        format!("latency-{}.csv", unix_now())
    }
}

pub fn latency_page(app: &LanPing) -> Element<'_, Message> {
    let latency = &app.latency;
    let content: Element<'_, Message> = match (&latency.error, latency.series.sent) {
        (Some(error), _) => empty_state("Cannot measure", error.to_string()),
        (None, 0) if latency.running => empty_state(
            "Waiting for the first answer",
            format!("Probing {}.", latency.target.trim()),
        ),
        (None, 0) => empty_state(
            "No measurement yet",
            "Enter a host name or an IPv4 address and press start.".to_owned(),
        ),
        (None, _) => results(latency),
    };
    let page = column![toolbar(latency), content]
        .spacing(SECTION_SPACING)
        .height(Length::Fill);
    page.into()
}

fn toolbar(latency: &LatencyState) -> Element<'_, Message> {
    let mut target = text_input("Host name or IPv4 address", &latency.target)
        .padding(theme::CONTROL_PADDING)
        .size(13)
        .width(Length::Fill)
        .style(theme::input_at(Edge::First));
    let mut interval = pick_list(
        Interval::ALL,
        Some(latency.interval),
        Message::LatencyIntervalPicked,
    )
    .text_size(13)
    .padding(theme::CONTROL_PADDING)
    .width(170)
    .style(theme::pick_at(Edge::Middle));
    if !latency.running {
        target = target
            .on_input(Message::LatencyTargetChanged)
            .on_submit(Message::LatencyStart);
    }
    interval = interval.placeholder("Interval");
    let action = if latency.running {
        with_tooltip(
            button(lucide::square().size(ICON_SIZE + 2.0))
                .on_press(Message::LatencyStop)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::danger_button)),
            "Stop",
        )
    } else {
        with_tooltip(
            button(lucide::play().size(ICON_SIZE + 2.0))
                .on_press(Message::LatencyStart)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::primary_button)),
            "Start",
        )
    };
    let mut status: Vec<Element<'_, Message>> = Vec::new();
    if let Some(prober) = &latency.prober {
        status.push(pill(
            format!("{} via {}", prober.address(), prober.method()),
            theme::ACCENT,
        ));
    }
    if latency.running {
        status.push(pill("Measuring", WARN));
    }
    status.push(space::horizontal().into());
    let has_data = latency.series.sent > 0;
    status.push(with_tooltip(
        icon_button(
            lucide::clipboard_copy().size(ICON_SIZE),
            has_data.then_some(Message::LatencyCopySummary),
            Edge::First,
        ),
        "Copy summary",
    ));
    status.push(with_tooltip(
        icon_button(
            lucide::download().size(ICON_SIZE),
            latency.has_log().then_some(Message::LatencySaveLog),
            Edge::Last,
        ),
        "Save log as CSV",
    ));
    column![
        input_group(vec![target.into(), interval.into(), action]),
        LayoutRow::with_children(status)
            .spacing(8)
            .align_y(Alignment::Center),
    ]
    .spacing(8)
    .into()
}

fn results(latency: &LatencyState) -> Element<'_, Message> {
    let series = &latency.series;
    let tiles = LayoutRow::with_children([
        tile(
            "Loss",
            format!("{:.1}%", series.loss_percent()),
            loss_color(series.loss_percent()),
        ),
        milliseconds_tile("Last", series.last_ms),
        milliseconds_tile("Average", series.average_ms()),
        milliseconds_tile("Best", series.best_ms),
        milliseconds_tile("Worst", series.worst_ms),
        tile(
            "Jitter",
            series
                .jitter_ms()
                .map_or_else(|| NO_VALUE.to_owned(), |ms| format!("{ms:.1} ms")),
            MUTED,
        ),
        tile("Probes", series.sent.to_string(), MUTED),
    ])
    .spacing(SECTION_SPACING)
    .wrap()
    .vertical_spacing(SECTION_SPACING);
    let history = series.history();
    let scale = history
        .iter()
        .flatten()
        .fold(MIN_GRAPH_SCALE_MS, |most, ms| most.max(*ms));
    let bars = history.into_iter().map(|sample| match sample {
        Some(ms) => Bar {
            fraction: Some((ms / scale) as f32),
            color: latency_color(ms),
        },
        None => Bar {
            fraction: None,
            color: BAD,
        },
    });
    column![
        tiles,
        text(format!("Last {GRAPH_LENGTH} probes, full height is {scale:.0} ms. A red dash is a probe with no answer."))
            .size(12)
            .color(MUTED),
        container(chart(bars, CHART_HEIGHT)).style(theme::card).padding(TILE_PADDING),
    ]
    .spacing(SECTION_SPACING)
    .into()
}

fn milliseconds_tile<'a>(label: &'static str, value: Option<f64>) -> Element<'a, Message> {
    match value {
        Some(ms) => tile(label, format!("{ms:.1} ms"), latency_color(ms)),
        None => tile(label, NO_VALUE.to_owned(), MUTED),
    }
}

fn tile<'a>(label: &'static str, value: String, color: iced::Color) -> Element<'a, Message> {
    container(
        column![
            text(label).size(12).color(MUTED),
            text(value).size(20).font(BOLD).color(color),
        ]
        .spacing(2),
    )
    .padding(TILE_PADDING)
    .width(TILE_WIDTH)
    .style(theme::card)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running_state() -> LatencyState {
        LatencyState {
            running: true,
            target: "example.com".to_owned(),
            ..LatencyState::default()
        }
    }

    #[test]
    fn intervals_last_one_five_and_thirty_seconds() {
        let seconds: Vec<u64> = Interval::ALL
            .iter()
            .map(|i| i.duration().as_secs())
            .collect();
        assert_eq!(seconds, [1, 5, 30]);
    }

    #[test]
    fn an_invalid_target_shows_an_error_and_does_not_start() {
        let mut state = LatencyState::default();
        state.set_target("-bad".to_owned());
        let _ = state.start();
        assert_eq!(state.error, Some(LatencyError::InvalidTarget));
        assert!(!state.running);
    }

    #[test]
    fn a_probe_is_recorded_while_running() {
        let mut state = running_state();
        let _ = state.sampled(0, Some(3.0));
        assert_eq!(state.series.sent, 1);
        assert!(state.has_log());
    }

    #[test]
    fn an_answer_from_a_stopped_run_is_ignored() {
        let mut state = running_state();
        state.stop();
        let _ = state.sampled(0, Some(3.0));
        assert_eq!(state.series.sent, 0);
    }

    #[test]
    fn a_failed_lookup_stops_the_run_with_its_error() {
        let mut state = running_state();
        let _ = state.ready(0, Err(LatencyError::NotFound));
        assert!(!state.running);
        assert_eq!(state.error, Some(LatencyError::NotFound));
    }

    #[test]
    fn a_run_with_probes_becomes_a_history_record_and_one_without_does_not() {
        let mut state = running_state();
        assert!(state.record_for_history().is_none());
        let _ = state.sampled(0, Some(4.0));
        let Some(Measurement::Latency(record)) = state.record_for_history() else {
            panic!("expected a latency record");
        };
        assert_eq!(record.target, "example.com");
        assert_eq!(record.probes, 1);
        assert_eq!(record.method, "unknown");
    }

    #[test]
    fn the_summary_names_the_target_and_the_counts() {
        let mut state = running_state();
        let _ = state.sampled(0, Some(4.0));
        let _ = state.sampled(0, None);
        assert_eq!(
            state.summary(),
            "example.com: 2 probes, 50.0% loss, last -, average 4.0 ms, best 4.0 ms, worst 4.0 ms, jitter -"
        );
    }
}
