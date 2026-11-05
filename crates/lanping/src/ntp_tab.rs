//! The Clock tab: how far this computer's clock is from a time server.

use iced::widget::{Row as LayoutRow, button, column, row, text, text_input};
use iced::{Alignment, Element, Length, Task};
use lanping_core::ntp::{NtpError, NtpReport, WELL_KNOWN_SERVERS, check};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, copy_button, lucide, with_tooltip};
use crate::job::Job;
use crate::results::pill;
use crate::settings_page::small_button;
use crate::theme::{self, BAD, Edge, GOOD, MUTED, SECTION_SPACING, WARN};
use crate::ui::{note, panel, scrolling};
use crate::view::input_group;

const LABEL_WIDTH: f32 = 130.0;
const INPUT_TEXT_SIZE: u32 = 13;
/// A clock further off than this can break certificate checks and logins.
const WARN_OFFSET_SECONDS: f64 = 1.0;
/// Closer than this to the server is as close as the measurement can tell.
const MATCH_OFFSET_SECONDS: f64 = 0.0005;
const MILLISECONDS: f64 = 1000.0;

enum Lookup {
    Idle,
    Running,
    Done(NtpReport),
    Failed(NtpError),
}

/// What is typed, and what the last check found.
pub struct NtpState {
    server: String,
    lookup: Lookup,
    job: Job,
}

impl NtpState {
    pub fn new() -> Self {
        Self {
            server: WELL_KNOWN_SERVERS[0].to_owned(),
            lookup: Lookup::Idle,
            job: Job::default(),
        }
    }

    pub fn set_server(&mut self, server: String) {
        self.server = server;
    }

    pub fn start(&mut self) -> Task<Message> {
        self.lookup = Lookup::Running;
        let server = self.server.clone();
        self.job
            .spawn(async move { check(&server).await }, Message::NtpDone)
    }

    pub fn finish(&mut self, generation: u64, result: Result<NtpReport, NtpError>) {
        if !self.job.is_current(generation) {
            return;
        }
        self.job.finished();
        self.lookup = match result {
            Ok(report) => Lookup::Done(report),
            Err(error) => Lookup::Failed(error),
        };
    }
}

pub fn ntp_panel(app: &LanPing) -> Element<'_, Message> {
    let ntp = &app.ntp;
    let busy = matches!(ntp.lookup, Lookup::Running);
    let mut server = text_input("Time server, such as pool.ntp.org", &ntp.server)
        .padding(theme::CONTROL_PADDING)
        .size(INPUT_TEXT_SIZE)
        .width(Length::Fill)
        .style(theme::input_at(Edge::First));
    if !busy {
        server = server
            .on_input(Message::NtpServerChanged)
            .on_submit(Message::NtpCheck);
    }
    let go = with_tooltip(
        button(lucide::alarm_clock_check().size(ICON_SIZE + 2.0))
            .on_press_maybe((!busy).then_some(Message::NtpCheck))
            .padding([7.0, 20.0])
            .style(theme::button_at(Edge::Last, theme::primary_button)),
        "Check the clock",
    );
    let suggestions = LayoutRow::with_children(WELL_KNOWN_SERVERS.map(|name| {
        small_button(
            name,
            (!busy).then(|| Message::NtpServerChanged(name.to_owned())),
        )
    }))
    .spacing(8)
    .wrap()
    .vertical_spacing(8);
    let result: Element<'_, Message> = match &ntp.lookup {
        Lookup::Idle => note("Press the button to ask the server what time it is."),
        Lookup::Running => note(format!("Asking {}.", ntp.server.trim())),
        Lookup::Failed(error) => text(error.to_string()).color(BAD).into(),
        Lookup::Done(report) => report_view(report),
    };
    let body =
        column![input_group(vec![server.into(), go]), suggestions, result].spacing(SECTION_SPACING);
    panel(scrolling(body))
}

fn report_view(report: &NtpReport) -> Element<'_, Message> {
    let off = report.clock_ahead_seconds.abs() >= WARN_OFFSET_SECONDS;
    let verdict = if off {
        pill("Clock is off", WARN)
    } else {
        pill("Clock is accurate", GOOD)
    };
    let mut lines = column![].spacing(6);
    for (label, value) in [
        ("This clock", describe_offset(report.clock_ahead_seconds)),
        ("Round trip", format_milliseconds(report.round_trip_seconds)),
        (
            "Server",
            format!("{}  ·  {}", report.server, report.address),
        ),
        ("Stratum", report.stratum.to_string()),
    ] {
        lines = lines.push(
            row![
                text(label).color(MUTED).width(LABEL_WIDTH),
                text(value.clone()).width(Length::Fill),
                copy_button(value, label),
            ]
            .spacing(8)
            .align_y(Alignment::Start),
        );
    }
    let mut page = column![verdict, lines].spacing(SECTION_SPACING);
    if off {
        page = page.push(note(
            "A clock this far off can make certificates look expired or not yet valid, and \
             can break logins that depend on the time.",
        ));
    }
    page.into()
}

/// `3.20 s ahead of the server`, `45 ms behind the server`, or `matches the server`.
fn describe_offset(clock_ahead_seconds: f64) -> String {
    let size = clock_ahead_seconds.abs();
    if size < MATCH_OFFSET_SECONDS {
        return "Matches the server".to_owned();
    }
    let direction = if clock_ahead_seconds > 0.0 {
        "ahead of"
    } else {
        "behind"
    };
    if size >= 1.0 {
        format!("{size:.2} s {direction} the server")
    } else {
        format!("{:.0} ms {direction} the server", size * MILLISECONDS)
    }
}

fn format_milliseconds(seconds: f64) -> String {
    format!("{:.0} ms", seconds * MILLISECONDS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_is_told_in_the_unit_that_fits_and_the_direction() {
        assert_eq!(describe_offset(0.0001), "Matches the server");
        assert_eq!(describe_offset(0.045), "45 ms ahead of the server");
        assert_eq!(describe_offset(-0.045), "45 ms behind the server");
        assert_eq!(describe_offset(3.2), "3.20 s ahead of the server");
        assert_eq!(describe_offset(-75.0), "75.00 s behind the server");
    }

    #[test]
    fn an_overtaken_answer_is_ignored() {
        let mut state = NtpState::new();
        let _ = state.start();
        let _ = state.start();
        state.finish(1, Err(NtpError::NoAnswer));
        assert!(matches!(state.lookup, Lookup::Running));
        state.finish(2, Err(NtpError::NoAnswer));
        assert!(matches!(state.lookup, Lookup::Failed(NtpError::NoAnswer)));
    }
}
