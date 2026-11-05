//! The MTU tab: the largest packet that reaches a host without being split.

use iced::widget::{button, column, row, text, text_input};
use iced::{Alignment, Element, Length, Task};
use lanping_core::path_mtu::{Method, MtuError, MtuReport, USUAL_MTUS, check, usual_name};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, lucide, with_tooltip};
use crate::job::Job;
use crate::results::pill;
use crate::theme::{self, ACCENT, BOLD, Edge, GOOD, MUTED, SECTION_SPACING, WARN};
use crate::ui::{card, empty_state, fact_row, note, page_header, scrolling};
use crate::view::input_group;

const LABEL_WIDTH: f32 = 130.0;
const HEADLINE_SIZE: u32 = 22;
const INPUT_TEXT_SIZE: u32 = 13;
/// The size of a standard Ethernet link; a path below it was narrowed by something.
const STANDARD_MTU: u32 = 1500;

enum Lookup {
    Idle,
    Running,
    Done(MtuReport),
    Failed(MtuError),
}

/// What is typed, and what the last measurement found.
pub struct MtuState {
    target: String,
    lookup: Lookup,
    job: Job,
}

impl MtuState {
    pub fn new() -> Self {
        Self {
            target: String::new(),
            lookup: Lookup::Idle,
            job: Job::default(),
        }
    }

    pub fn set_target(&mut self, target: String) {
        self.target = target;
    }

    pub fn start(&mut self) -> Task<Message> {
        self.lookup = Lookup::Running;
        let target = self.target.clone();
        self.job
            .spawn(async move { check(&target).await }, Message::MtuDone)
    }

    pub fn finish(&mut self, generation: u64, result: Result<MtuReport, MtuError>) {
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

/// The MTU page of the DNS tab.
pub fn mtu_page(app: &LanPing) -> Element<'_, Message> {
    let mtu = &app.mtu;
    let busy = matches!(mtu.lookup, Lookup::Running);
    let mut target = text_input("Host name or address, such as example.com", &mtu.target)
        .padding(theme::CONTROL_PADDING)
        .size(INPUT_TEXT_SIZE)
        .width(Length::Fill)
        .style(theme::input_at(Edge::First));
    if !busy {
        target = target
            .on_input(Message::MtuTargetChanged)
            .on_submit(Message::MtuCheck);
    }
    let go = with_tooltip(
        button(lucide::ruler().size(ICON_SIZE + 2.0))
            .on_press_maybe((!busy).then_some(Message::MtuCheck))
            .padding([7.0, 20.0])
            .style(theme::button_at(Edge::Last, theme::primary_button)),
        "Measure the path",
    );
    let header = page_header(
        "Path MTU",
        "The largest packet that reaches a host without being split. A low one stalls big transfers.",
        None,
    );
    let content: Element<'_, Message> = match &mtu.lookup {
        Lookup::Idle => empty_state(
            "Not measured yet",
            "Enter a host and press the button. It uses the system's ping or tracepath.",
        ),
        Lookup::Running => empty_state(
            "Measuring",
            format!(
                "Sending test packets to {}. This can take up to half a minute.",
                mtu.target.trim()
            ),
        ),
        Lookup::Failed(error) => empty_state("Cannot measure the path", error.to_string()),
        Lookup::Done(report) => scrolling(report_view(report)).into(),
    };
    column![header, input_group(vec![target.into(), go]), content]
        .spacing(SECTION_SPACING)
        .height(Length::Fill)
        .into()
}

fn report_view(report: &MtuReport) -> Element<'_, Message> {
    let size = if report.at_least {
        format!("{} bytes or more", report.mtu)
    } else {
        format!("{} bytes", report.mtu)
    };
    let verdict = match (usual_name(report.mtu), report.mtu.cmp(&STANDARD_MTU)) {
        (Some(name), std::cmp::Ordering::Equal) => pill(name, GOOD),
        (Some(name), _) => pill(name, ACCENT),
        (None, std::cmp::Ordering::Less) => pill("Smaller than usual", WARN),
        (None, _) => pill("Larger than Ethernet", ACCENT),
    };
    let headline = row![text(size).size(HEADLINE_SIZE).font(BOLD), verdict]
        .spacing(12)
        .align_y(Alignment::Center);
    let lines = details(report)
        .into_iter()
        .fold(column![].spacing(6), |list, (label, value)| {
            list.push(fact_row(label, value))
        });
    card(
        column![
            headline,
            lines,
            comparison(report.mtu),
            note(advice(report))
        ]
        .spacing(SECTION_SPACING),
    )
}

/// The labelled facts of a measurement.
fn details(report: &MtuReport) -> Vec<(&'static str, String)> {
    let method = match report.method {
        Method::Tracepath => "tracepath",
        Method::PingSearch => "ping with the do-not-fragment flag",
    };
    let mut lines = vec![
        ("Host", report.target.clone()),
        ("Measured with", method.to_owned()),
    ];
    if let Some(hop) = &report.narrowed_at {
        let place = match &hop.address {
            Some(address) => format!("hop {} ({address})", hop.number),
            None => format!("hop {}", hop.number),
        };
        lines.push(("Narrowed at", place));
    }
    lines
}

/// The usual sizes, with the one that was found marked.
fn comparison(found: u32) -> Element<'static, Message> {
    USUAL_MTUS
        .iter()
        .fold(
            column![text("Usual sizes").size(13).color(MUTED)].spacing(4),
            |list, (mtu, name)| {
                let marker = if *mtu == found { "◀ this path" } else { "" };
                list.push(
                    row![
                        text(mtu.to_string()).size(13).font(BOLD).width(LABEL_WIDTH),
                        text(*name).size(13).width(Length::Fill),
                        text(marker).size(13).color(ACCENT),
                    ]
                    .spacing(8),
                )
            },
        )
        .into()
}

/// What a result means, in plain words.
fn advice(report: &MtuReport) -> String {
    if report.at_least {
        return "A full-size packet got through, so the path is not the cause of slow or \
                stalled connections."
            .to_owned();
    }
    if report.mtu >= STANDARD_MTU {
        return "The path carries standard full-size packets.".to_owned();
    }
    let place = report.narrowed_at.as_ref().map_or(String::new(), |hop| {
        format!(" Something at hop {} lowered it.", hop.number)
    });
    format!(
        "Packets larger than {} bytes are split or dropped on this path.{place} A VPN, a tunnel or a \
         PPPoE link usually does this. If connections stall on large transfers, set the MTU of that \
         link to this value or lower.",
        report.mtu
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanping_core::path_mtu::Hop;

    fn report(mtu: u32, at_least: bool, hop: Option<Hop>) -> MtuReport {
        MtuReport {
            target: "example.com".to_owned(),
            mtu,
            method: Method::Tracepath,
            narrowed_at: hop,
            at_least,
        }
    }

    #[test]
    fn advice_says_what_a_narrow_path_means_and_where() {
        let hop = Hop {
            number: 2,
            address: Some("10.0.0.1".to_owned()),
        };
        let text = advice(&report(1492, false, Some(hop)));
        assert!(text.contains("1492") && text.contains("hop 2"), "{text}");
        assert!(advice(&report(1500, true, None)).contains("not the cause"));
        assert!(advice(&report(1500, false, None)).contains("standard"));
    }

    #[test]
    fn the_details_name_the_method_and_the_narrowing_hop() {
        let hop = Hop {
            number: 3,
            address: None,
        };
        let lines = details(&report(1400, false, Some(hop)));
        assert!(lines.contains(&("Measured with", "tracepath".to_owned())));
        assert!(lines.contains(&("Narrowed at", "hop 3".to_owned())));
    }

    #[test]
    fn an_overtaken_answer_is_ignored() {
        let mut state = MtuState::new();
        let _ = state.start();
        let _ = state.start();
        state.finish(1, Err(MtuError::NoAnswer));
        assert!(matches!(state.lookup, Lookup::Running));
        state.finish(2, Err(MtuError::NoAnswer));
        assert!(matches!(state.lookup, Lookup::Failed(MtuError::NoAnswer)));
    }
}
