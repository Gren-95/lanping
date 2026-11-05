//! The TLS tab: the certificate a server presents, and whether the system would trust it.

use iced::widget::{Row as LayoutRow, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length, Task};
use lanping_core::tls::{CertInfo, TlsError, TlsReport, Trust, check};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, copy_button, lucide, with_tooltip};
use crate::job::Job;
use crate::results::pill;
use crate::theme::{self, ACCENT, BAD, BOLD, Edge, GOOD, MUTED, SECTION_SPACING, WARN};
use crate::timestamp::format_unix_utc;
use crate::ui::{note, scrolling};
use crate::view::input_group;

const LABEL_WIDTH: f32 = 130.0;
const CARD_PADDING: f32 = 14.0;
/// A certificate with fewer days left than this is shown in amber.
const EXPIRY_WARNING_DAYS: i64 = 30;

enum Lookup {
    Idle,
    Running,
    Done(TlsReport),
    Failed(TlsError),
}

/// What is typed, and what the last check found.
pub struct TlsState {
    target: String,
    /// The host name to ask for when it differs from the address. Empty for none.
    server_name: String,
    lookup: Lookup,
    job: Job,
}

impl TlsState {
    pub fn new() -> Self {
        Self {
            target: String::new(),
            server_name: String::new(),
            lookup: Lookup::Idle,
            job: Job::default(),
        }
    }

    pub fn set_target(&mut self, target: String) {
        self.target = target;
    }

    pub fn set_server_name(&mut self, server_name: String) {
        self.server_name = server_name;
    }

    pub fn start(&mut self) -> Task<Message> {
        self.lookup = Lookup::Running;
        let target = self.target.clone();
        let server_name = self.server_name.clone();
        self.job.spawn(
            async move { check(&target, Some(&server_name)).await },
            Message::TlsDone,
        )
    }

    pub fn finish(&mut self, generation: u64, result: Result<TlsReport, TlsError>) {
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

/// Windows at least this wide keep the inputs on one line.
const WIDE_WIDTH: f32 = 820.0;
const NAME_INPUT_WIDTH: f32 = 260.0;
const INPUT_TEXT_SIZE: u32 = 13;

pub fn tls_page(app: &LanPing, width: f32) -> Element<'_, Message> {
    let tls = &app.tls;
    let busy = matches!(tls.lookup, Lookup::Running);
    let wide = width >= WIDE_WIDTH;
    let target_edge = if wide { Edge::First } else { Edge::Alone };
    let mut target = text_input(
        "Host name or address, and a port if it is not 443",
        &tls.target,
    )
    .padding(theme::CONTROL_PADDING)
    .size(INPUT_TEXT_SIZE)
    .width(Length::Fill)
    .style(theme::input_at(target_edge));
    let name_edge = if wide { Edge::Middle } else { Edge::First };
    let mut name = text_input("Server name (optional)", &tls.server_name)
        .padding(theme::CONTROL_PADDING)
        .size(INPUT_TEXT_SIZE)
        .width(if wide {
            Length::Fixed(NAME_INPUT_WIDTH)
        } else {
            Length::Fill
        })
        .style(theme::input_at(name_edge));
    if !busy {
        target = target
            .on_input(Message::TlsTargetChanged)
            .on_submit(Message::TlsCheck);
        name = name
            .on_input(Message::TlsServerNameChanged)
            .on_submit(Message::TlsCheck);
    }
    let go = with_tooltip(
        button(lucide::shield_check().size(ICON_SIZE + 2.0))
            .on_press_maybe((!busy).then_some(Message::TlsCheck))
            .padding([7.0, 20.0])
            .style(theme::button_at(Edge::Last, theme::primary_button)),
        "Check certificate",
    );
    let inputs: Element<'_, Message> = if wide {
        input_group(vec![target.into(), name.into(), go])
    } else {
        column![target, input_group(vec![name.into(), go])]
            .spacing(8)
            .into()
    };
    let result: Element<'_, Message> = match &tls.lookup {
        Lookup::Idle => {
            note("Enter a host, such as example.com or 192.168.0.1:8443, and press the button.")
        }
        Lookup::Running => note(format!("Connecting to {}.", tls.target.trim())),
        Lookup::Failed(error) => text(error.to_string()).color(BAD).into(),
        Lookup::Done(report) => report_view(report),
    };
    let body = column![inputs, result].spacing(SECTION_SPACING);
    scrolling(body).into()
}

fn report_view(report: &TlsReport) -> Element<'_, Message> {
    let mut verdict: Vec<Element<'_, Message>> = vec![match &report.trust {
        Trust::Trusted => pill("Trusted", GOOD),
        Trust::SelfSigned => pill("Self-signed", WARN),
        Trust::Problem(reason) => pill(reason.clone(), BAD),
    }];
    if let Some(leaf) = report.chain.first() {
        verdict.push(expiry_pill(leaf.days_left(report.checked_at)));
    }
    let mut connection = format!("{}:{}", report.host, report.port);
    if let Some(address) = report.address {
        connection.push_str(&format!("  ·  {}", address.ip()));
    }
    if let Some(protocol) = &report.protocol {
        connection.push_str(&format!("  ·  {protocol}"));
    }
    if let Some(cipher) = &report.cipher {
        connection.push_str(&format!("  ·  {cipher}"));
    }
    let cards = report.chain.iter().enumerate().fold(
        column![].spacing(SECTION_SPACING),
        |list, (index, certificate)| {
            list.push(certificate_card(index, certificate, report.checked_at))
        },
    );
    column![
        LayoutRow::with_children(verdict)
            .spacing(8)
            .align_y(Alignment::Center)
            .wrap()
            .vertical_spacing(8),
        text(connection).size(12).color(MUTED),
        cards,
    ]
    .spacing(SECTION_SPACING)
    .into()
}

fn expiry_pill<'a>(days_left: i64) -> Element<'a, Message> {
    match days_left {
        ..=-1 => pill(format!("Expired {} days ago", -days_left), BAD),
        0..=EXPIRY_WARNING_DAYS => pill(format!("Expires in {days_left} days"), WARN),
        _ => pill(format!("Expires in {days_left} days"), ACCENT),
    }
}

fn certificate_card(index: usize, certificate: &CertInfo, checked_at: i64) -> Element<'_, Message> {
    let title = if index == 0 {
        "Server certificate".to_owned()
    } else {
        format!("Chain certificate {}", index + 1)
    };
    let names = certificate.names.join(", ");
    let rows = [
        ("Subject", certificate.subject.clone()),
        ("Issuer", certificate.issuer.clone()),
        ("Valid from", format_seconds(certificate.not_before)),
        (
            "Valid until",
            format!(
                "{}  ({} days left)",
                format_seconds(certificate.not_after),
                certificate.days_left(checked_at)
            ),
        ),
        ("Serial number", certificate.serial.clone()),
        (
            "Valid for",
            if names.is_empty() {
                "-".to_owned()
            } else {
                names
            },
        ),
    ];
    let lines = rows
        .into_iter()
        .fold(column![].spacing(6), |list, (label, value)| {
            let copy = copy_button(value.clone(), label);
            list.push(
                row![
                    text(label).color(MUTED).width(LABEL_WIDTH),
                    text(value).width(Length::Fill),
                    copy,
                ]
                .spacing(8)
                .align_y(Alignment::Start),
            )
        });
    container(column![text(title).size(15).font(BOLD), lines].spacing(10))
        .padding(CARD_PADDING)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

fn format_seconds(unix_seconds: i64) -> String {
    format_unix_utc(u64::try_from(unix_seconds).unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overtaken_answer_is_ignored() {
        let mut state = TlsState::new();
        let _ = state.start();
        let _ = state.start();
        state.finish(1, Err(TlsError::Timeout));
        assert!(matches!(state.lookup, Lookup::Running));
        state.finish(2, Err(TlsError::Timeout));
        assert!(matches!(state.lookup, Lookup::Failed(TlsError::Timeout)));
    }

    #[test]
    fn dates_before_1970_show_as_the_epoch() {
        assert_eq!(format_seconds(-5), format_seconds(0));
    }
}
