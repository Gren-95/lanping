//! The WHOIS page of the DNS tab: who is behind a domain name or an IP address.

use iced::font::Weight;
use iced::widget::{button, column, container, row, space, text, text_input};
use iced::{Alignment, Element, Font, Length, Task};
use lanping_core::whois::{WhoisError, WhoisReport, whois};

use crate::app::Message;
use crate::icons::{ICON_SIZE, copy_button, icon_button, lucide, with_tooltip};
use crate::job::Job;
use crate::theme::{self, BAD, Edge, MUTED};
use crate::view::input_group;

const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};
const SECTION_SPACING: f32 = 14.0;
const LABEL_WIDTH: f32 = 130.0;
const ROW_PADDING: [f32; 2] = [8.0, 12.0];
const RAW_TEXT_SIZE: f32 = 12.0;
const CONTROL_TEXT_SIZE: u32 = 13;

enum Lookup {
    Idle,
    Running,
    Done(WhoisReport),
    Failed(WhoisError),
}

/// What is typed, and what the last lookup found.
pub struct WhoisState {
    query: String,
    lookup: Lookup,
    job: Job,
}

impl WhoisState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            lookup: Lookup::Idle,
            job: Job::default(),
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.query = query;
    }

    pub fn start(&mut self) -> Task<Message> {
        self.lookup = Lookup::Running;
        let query = self.query.clone();
        self.job
            .spawn(async move { whois(&query).await }, Message::WhoisDone)
    }

    pub fn finish(&mut self, generation: u64, result: Result<WhoisReport, WhoisError>) {
        if !self.job.is_current(generation) {
            return;
        }
        self.job.finished();
        self.lookup = match result {
            Ok(report) => Lookup::Done(report),
            Err(error) => Lookup::Failed(error),
        };
    }

    /// The full answer of the last lookup, for the clipboard.
    pub fn answer_text(&self) -> Option<&str> {
        match &self.lookup {
            Lookup::Done(report) => Some(&report.text),
            _ => None,
        }
    }
}

pub fn whois_view(state: &WhoisState) -> Element<'_, Message> {
    let busy = matches!(state.lookup, Lookup::Running);
    let mut query = text_input("Domain name or IP address", &state.query)
        .padding(theme::CONTROL_PADDING)
        .size(CONTROL_TEXT_SIZE)
        .width(Length::Fill)
        .style(theme::input_at(Edge::First));
    if !busy {
        query = query
            .on_input(Message::WhoisQueryChanged)
            .on_submit(Message::WhoisLookup);
    }
    let go = with_tooltip(
        button(lucide::search().size(ICON_SIZE + 2.0))
            .on_press_maybe((!busy).then_some(Message::WhoisLookup))
            .padding([7.0, 20.0])
            .style(theme::button_at(Edge::Last, theme::primary_button)),
        "Look up",
    );
    let result: Element<'_, Message> = match &state.lookup {
        Lookup::Idle => note("Enter a domain name or an IP address and press the search button."),
        Lookup::Running => note("Asking the WHOIS servers."),
        Lookup::Failed(error) => text(error.to_string()).color(BAD).into(),
        Lookup::Done(report) => report_view(report),
    };
    column![input_group(vec![query.into(), go]), result]
        .spacing(SECTION_SPACING)
        .into()
}

fn note<'a>(message: &'static str) -> Element<'a, Message> {
    text(message).size(14).color(MUTED).into()
}

fn report_view(report: &WhoisReport) -> Element<'_, Message> {
    let header = row![
        text(format!("Answer from {}", report.server))
            .size(16)
            .font(BOLD),
        space::horizontal(),
        with_tooltip(
            icon_button(
                lucide::clipboard_copy().size(ICON_SIZE),
                Some(Message::WhoisCopy),
                Edge::Alone,
            ),
            "Copy the full answer",
        ),
    ]
    .align_y(Alignment::Center);
    let summary = report
        .summary
        .iter()
        .enumerate()
        .fold(column![], |list, (index, line)| {
            let cells = row![
                text(line.label).color(MUTED).width(LABEL_WIDTH),
                text(line.value.as_str()).font(BOLD),
                copy_button(line.value.clone(), line.label),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            list.push(
                container(cells)
                    .padding(ROW_PADDING)
                    .width(Length::Fill)
                    .style(theme::stripe(index % 2 == 1)),
            )
        });
    let raw = container(
        text(report.text.as_str())
            .size(RAW_TEXT_SIZE)
            .font(Font::MONOSPACE),
    )
    .padding(ROW_PADDING)
    .width(Length::Fill)
    .style(theme::card);
    column![header, summary, raw]
        .spacing(SECTION_SPACING)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> WhoisReport {
        WhoisReport {
            server: "whois.example.net".to_owned(),
            text: "answer".to_owned(),
            summary: Vec::new(),
        }
    }

    #[test]
    fn an_overtaken_answer_is_ignored() {
        let mut state = WhoisState::new();
        let _ = state.start();
        let _ = state.start();
        state.finish(1, Ok(report()));
        assert!(state.answer_text().is_none());
        state.finish(2, Ok(report()));
        assert_eq!(state.answer_text(), Some("answer"));
    }

    #[test]
    fn a_failure_leaves_no_answer_to_copy() {
        let mut state = WhoisState::new();
        let _ = state.start();
        state.finish(1, Err(WhoisError::Timeout));
        assert!(state.answer_text().is_none());
    }
}
