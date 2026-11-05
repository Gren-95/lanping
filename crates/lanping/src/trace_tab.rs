//! The Trace tab: a traceroute that repeats, with loss and latency per hop.

use std::time::Duration;

use iced::widget::{
    Row as LayoutRow, button, column, container, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Length, Task};
use lanping_core::trace::{
    HopSample, HopStats, TraceError, TraceStats, run_round, stats_as_text, validate_target,
};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, copy_button, icon_button, lucide, with_tooltip};
use crate::job::Job;
use crate::results::pill;
use crate::sparkline::{Bar, sparkline, width_of};
use crate::table_grid::{cell_width, grid_line};
use crate::theme::{
    self, BAD, BOLD, Edge, MIN_GRAPH_SCALE_MS, MUTED, NO_VALUE, SCROLLBAR_GAP, SECTION_SPACING,
    WARN, latency_color, loss_color,
};
use crate::ui::empty_state;
use crate::view::input_group;

/// Rest between two rounds, so a nearby target does not start a process every few milliseconds.
const ROUND_PAUSE: Duration = Duration::from_secs(1);
const UNKNOWN_ADDRESS: &str = "???";

/// One round after a rest.
async fn round(target: String, pause: Duration) -> Result<Vec<HopSample>, TraceError> {
    tokio::time::sleep(pause).await;
    run_round(&target).await
}

/// What the running or finished trace knows.
#[derive(Debug, Default)]
pub struct TraceState {
    target: String,
    /// The target of the trace on screen; editing the field does not change it.
    traced: String,
    stats: TraceStats,
    running: bool,
    error: Option<TraceError>,
    job: Job,
}

impl TraceState {
    pub fn set_target(&mut self, target: String) {
        self.target = target;
    }

    /// Starts over against the typed target.
    pub fn start(&mut self) -> Task<Message> {
        let target = match validate_target(&self.target) {
            Ok(target) => target.to_owned(),
            Err(error) => {
                self.error = Some(error);
                return Task::none();
            }
        };
        self.stop();
        self.traced = target;
        self.stats = TraceStats::default();
        self.error = None;
        self.running = true;
        self.job.spawn(
            round(self.traced.clone(), Duration::ZERO),
            Message::TraceRound,
        )
    }

    pub fn stop(&mut self) {
        self.running = false;
        self.job.cancel();
    }

    /// Records a finished round and starts the next one.
    pub fn finish_round(
        &mut self,
        generation: u64,
        result: Result<Vec<HopSample>, TraceError>,
    ) -> Task<Message> {
        if !self.job.is_current(generation) || !self.running {
            return Task::none();
        }
        match result {
            Ok(samples) => {
                self.stats.record_round(&samples);
                self.job
                    .spawn_next(round(self.traced.clone(), ROUND_PAUSE), Message::TraceRound)
            }
            Err(error) => {
                self.error = Some(error);
                self.running = false;
                self.job.finished();
                Task::none()
            }
        }
    }

    /// The statistics as tab-separated text, and how many hops there are.
    pub fn stats_as_text(&self) -> (String, usize) {
        (stats_as_text(&self.stats), self.stats.hops().len())
    }
}

pub fn trace_page(app: &LanPing) -> Element<'_, Message> {
    let trace = &app.trace;
    let content: Element<'_, Message> = match (&trace.error, trace.stats.hops().is_empty()) {
        (Some(error), _) => empty_state("Cannot trace", error.to_string()),
        (None, true) if trace.running => {
            empty_state("Tracing", format!("Finding the path to {}.", trace.traced))
        }
        (None, true) => empty_state(
            "No trace yet",
            "Enter a host name or an IP address and press start.".to_owned(),
        ),
        (None, false) => hop_table(&trace.stats),
    };
    let page = column![toolbar(trace), content]
        .spacing(SECTION_SPACING)
        .height(Length::Fill);
    page.into()
}

fn toolbar(trace: &TraceState) -> Element<'_, Message> {
    let mut target = text_input("Host name or IP address", &trace.target)
        .padding(theme::CONTROL_PADDING)
        .size(13)
        .width(Length::Fill)
        .style(theme::input_at(Edge::First));
    if !trace.running {
        target = target
            .on_input(Message::TraceTargetChanged)
            .on_submit(Message::TraceStart);
    }
    let action = if trace.running {
        with_tooltip(
            button(lucide::square().size(ICON_SIZE + 2.0))
                .on_press(Message::TraceStop)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::danger_button)),
            "Stop trace",
        )
    } else {
        with_tooltip(
            button(lucide::play().size(ICON_SIZE + 2.0))
                .on_press(Message::TraceStart)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::primary_button)),
            "Start trace",
        )
    };
    let mut status: Vec<Element<'_, Message>> = Vec::new();
    if trace.running {
        status.push(pill("Tracing", WARN));
    }
    if trace.stats.rounds() > 0 {
        status.push(pill(
            format!("{} rounds", trace.stats.rounds()),
            theme::ACCENT,
        ));
    }
    status.push(space::horizontal().into());
    status.push(with_tooltip(
        icon_button(
            lucide::clipboard_copy().size(ICON_SIZE),
            (!trace.stats.hops().is_empty()).then_some(Message::TraceCopyRows),
            Edge::Alone,
        ),
        "Copy hops",
    ));
    column![
        input_group(vec![target.into(), action]),
        LayoutRow::with_children(status)
            .spacing(8)
            .align_y(Alignment::Center),
    ]
    .spacing(8)
    .into()
}

#[derive(Clone, Copy)]
struct Column {
    title: &'static str,
    width: f32,
}

const COLUMNS: [Column; 9] = [
    Column {
        title: "Hop",
        width: 40.0,
    },
    Column {
        title: "Address",
        width: 190.0,
    },
    Column {
        title: "Loss",
        width: 64.0,
    },
    Column {
        title: "Sent",
        width: 50.0,
    },
    Column {
        title: "Last",
        width: 70.0,
    },
    Column {
        title: "Average",
        width: 70.0,
    },
    Column {
        title: "Best",
        width: 70.0,
    },
    Column {
        title: "Worst",
        width: 70.0,
    },
    Column {
        title: "Jitter",
        width: 70.0,
    },
];

fn graph_width() -> f32 {
    cell_width(width_of(HopStats::HISTORY_LENGTH))
}

fn hop_table(stats: &TraceStats) -> Element<'_, Message> {
    let mut header_cells: Vec<(f32, Element<'_, Message>)> = COLUMNS
        .iter()
        .map(|column| {
            (
                cell_width(column.width),
                text(column.title).size(13).color(MUTED).into(),
            )
        })
        .collect();
    header_cells.push((graph_width(), text("Latency").size(13).color(MUTED).into()));
    let scale = graph_scale(stats);
    let lines = stats.hops().iter().enumerate().fold(
        column![grid_line(header_cells)],
        |list, (index, hop)| {
            list.push(container(grid_line(cells(hop, scale))).style(theme::stripe(index % 2 == 1)))
        },
    );
    scrollable(
        container(lines).padding(
            iced::Padding::ZERO
                .right(SCROLLBAR_GAP)
                .bottom(SCROLLBAR_GAP),
        ),
    )
    .direction(scrollable::Direction::Both {
        vertical: scrollable::Scrollbar::default(),
        horizontal: scrollable::Scrollbar::default(),
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

/// Round trip that fills a whole bar: the slowest recent answer on any hop.
fn graph_scale(stats: &TraceStats) -> f64 {
    stats
        .hops()
        .iter()
        .flat_map(|hop| hop.history())
        .flatten()
        .fold(MIN_GRAPH_SCALE_MS, f64::max)
}

fn cells(hop: &HopStats, scale: f64) -> Vec<(f32, Element<'_, Message>)> {
    let address: Element<'_, Message> = match hop.address {
        Some(address) => {
            let value = address.to_string();
            row![text(value.clone()), copy_button(value, "address")]
                .spacing(8)
                .align_y(Alignment::Center)
                .into()
        }
        None => text(UNKNOWN_ADDRESS).color(MUTED).into(),
    };
    let values: [Element<'_, Message>; 9] = [
        text(hop.hop.to_string()).font(BOLD).into(),
        address,
        pill(
            format!("{:.0}%", hop.loss_percent()),
            loss_color(hop.loss_percent()),
        ),
        text(hop.sent.to_string()).into(),
        milliseconds(hop.last_ms),
        milliseconds(hop.average_ms()),
        milliseconds(hop.best_ms),
        milliseconds(hop.worst_ms),
        plain_milliseconds(hop.jitter_ms()),
    ];
    let mut cells: Vec<(f32, Element<'_, Message>)> = COLUMNS
        .iter()
        .map(|column| cell_width(column.width))
        .zip(values)
        .collect();
    cells.push((graph_width(), latency_graph(hop, scale)));
    cells
}

fn milliseconds<'a>(value: Option<f64>) -> Element<'a, Message> {
    match value {
        Some(ms) => text(format!("{ms:.1} ms")).color(latency_color(ms)).into(),
        None => text(NO_VALUE).color(MUTED).into(),
    }
}

fn plain_milliseconds<'a>(value: Option<f64>) -> Element<'a, Message> {
    match value {
        Some(ms) => text(format!("{ms:.1} ms")).into(),
        None => text(NO_VALUE).color(MUTED).into(),
    }
}

/// One bar per recent round, as tall as the round trip. A red dash is a round with no answer.
fn latency_graph(hop: &HopStats, scale: f64) -> Element<'_, Message> {
    sparkline(hop.history().into_iter().map(|sample| match sample {
        Some(ms) => Bar {
            fraction: Some((ms / scale) as f32),
            color: latency_color(ms),
        },
        None => Bar {
            fraction: None,
            color: BAD,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(hop: u8, rtt_ms: Option<f64>) -> HopSample {
        HopSample {
            hop,
            address: rtt_ms.map(|_| "10.0.0.1".parse().unwrap()),
            rtt_ms,
        }
    }

    fn running_state() -> TraceState {
        TraceState {
            running: true,
            traced: "example.com".to_owned(),
            ..TraceState::default()
        }
    }

    #[test]
    fn graph_scale_never_drops_below_the_minimum() {
        let mut stats = TraceStats::default();
        stats.record_round(&[sample(1, Some(1.0))]);
        assert_eq!(graph_scale(&stats), MIN_GRAPH_SCALE_MS);
        stats.record_round(&[sample(1, Some(80.0))]);
        assert_eq!(graph_scale(&stats), 80.0);
    }

    #[test]
    fn an_invalid_target_shows_an_error_and_does_not_start() {
        let mut state = TraceState::default();
        state.set_target("-bad".to_owned());
        let _ = state.start();
        assert_eq!(state.error, Some(TraceError::InvalidTarget));
        assert!(!state.running);
    }

    #[test]
    fn a_round_is_recorded_while_running() {
        let mut state = running_state();
        let _ = state.finish_round(0, Ok(vec![sample(1, Some(2.0))]));
        assert_eq!(state.stats.rounds(), 1);
    }

    #[test]
    fn an_answer_from_a_stopped_trace_is_ignored() {
        let mut state = running_state();
        state.stop();
        let _ = state.finish_round(0, Ok(vec![sample(1, Some(2.0))]));
        assert_eq!(state.stats.rounds(), 0);
    }

    #[test]
    fn a_failed_round_stops_the_trace_with_its_error() {
        let mut state = running_state();
        let _ = state.finish_round(0, Err(TraceError::TimedOut));
        assert!(!state.running);
        assert_eq!(state.error, Some(TraceError::TimedOut));
    }
}
