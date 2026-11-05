//! The Syslog tab: start and stop the listener, tell a device where to send, and watch the
//! messages arrive.

use iced::widget::{Row as LayoutRow, button, checkbox, column, pick_list, row, text, text_input};
use iced::{Alignment, Element, Font, Length};
use lanping_core::syslog::{STANDARD_PORT, Severity, SyslogMessage};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, lucide};
use crate::results::pill;
use crate::settings_page::small_button;
use crate::syslog_tab::{SHOWN_MESSAGES, SeverityChoice, Status, SyslogState};
use crate::theme::{self, ACCENT, BAD, BOLD, Edge, MUTED, SECTION_SPACING, WARN};
use crate::timestamp::format_unix_time_utc;
use crate::ui::{card, empty_state, note, page_header, panel, scrolling};
use crate::view::input_group;

const INPUT_TEXT_SIZE: u32 = 13;
const PORT_WIDTH: f32 = 90.0;
const FILTER_WIDTH: f32 = 180.0;
const SEVERITY_WIDTH: f32 = 170.0;
const TIME_WIDTH: f32 = 70.0;
const SEVERITY_COLUMN_WIDTH: f32 = 80.0;
const HOST_WIDTH: f32 = 140.0;
const APP_WIDTH: f32 = 90.0;
const ROW_TEXT_SIZE: u32 = 12;

pub fn syslog_panel(app: &LanPing) -> Element<'_, Message> {
    let syslog = &app.syslog;
    let header = page_header(
        "Syslog",
        "Log messages that routers, NAS boxes and other devices send to this computer.",
        Some(start_stop(syslog)),
    );
    let mut page = column![header, setup_row(syslog)].spacing(SECTION_SPACING);
    if let Status::Listening {
        port,
        tcp,
        addresses,
    } = &syslog.status
    {
        page = page.push(listening_card(*port, *tcp, addresses));
    }
    if let Status::Failed(reason) = &syslog.status {
        page = page.push(text(reason.as_str()).color(BAD));
    }
    page = page.push(filter_row(syslog));
    panel(page.push(messages_view(syslog)).height(Length::Fill))
}

fn start_stop(syslog: &SyslogState) -> Element<'_, Message> {
    let (icon, label, message) = if syslog.listening() {
        (lucide::square(), "Stop listening", Message::SyslogStop)
    } else {
        (lucide::play(), "Start listening", Message::SyslogStart)
    };
    button(
        row![icon.size(ICON_SIZE), text(label).size(14)]
            .spacing(8)
            .align_y(Alignment::Center),
    )
    .on_press(message)
    .padding([7.0, 20.0])
    .style(theme::primary_button)
    .into()
}

fn setup_row(syslog: &SyslogState) -> Element<'_, Message> {
    let locked = syslog.listening();
    let mut port = text_input("Port", &syslog.port)
        .padding(theme::CONTROL_PADDING)
        .size(INPUT_TEXT_SIZE)
        .width(PORT_WIDTH);
    let mut tcp = checkbox(syslog.tcp).label("Also listen over TCP").size(16);
    if !locked {
        port = port
            .on_input(Message::SyslogPortChanged)
            .on_submit(Message::SyslogStart);
        tcp = tcp.on_toggle(Message::SyslogTcpToggled);
    }
    row![text("Port").color(MUTED), port, tcp]
        .spacing(12)
        .align_y(Alignment::Center)
        .into()
}

/// Where the listener is and how to point a device at it.
fn listening_card<'a>(
    port: u16,
    tcp: bool,
    addresses: &[std::net::IpAddr],
) -> Element<'a, Message> {
    let protocols = if tcp { "UDP and TCP" } else { "UDP" };
    let places = if addresses.is_empty() {
        "this computer".to_owned()
    } else {
        addresses
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut body = column![
        row![
            pill("Listening", ACCENT),
            text(format!(
                "{protocols} port {port} on all addresses of this computer: {places}"
            ))
            .size(13)
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        note(format!(
            "In the device's log settings, set the remote syslog server to one of these addresses \
             and port {port}."
        )),
    ]
    .spacing(6);
    if port != STANDARD_PORT {
        body = body.push(note(
            "Devices send to port 514 unless told otherwise. Port 514 needs administrator rights \
             here, so this uses a higher port: set the port on the device as well.",
        ));
    }
    card(body)
}

fn filter_row(syslog: &SyslogState) -> Element<'_, Message> {
    let host = text_input("Host or address", &syslog.filter.host)
        .on_input(Message::SyslogHostFilter)
        .padding(theme::CONTROL_PADDING)
        .size(INPUT_TEXT_SIZE)
        .width(FILTER_WIDTH)
        .style(theme::input_at(Edge::First));
    let search = text_input("Search messages", &syslog.filter.text)
        .on_input(Message::SyslogTextFilter)
        .padding(theme::CONTROL_PADDING)
        .size(INPUT_TEXT_SIZE)
        .width(Length::Fill)
        .style(theme::input_at(Edge::Last));
    let severity = pick_list(
        SeverityChoice::all(),
        Some(syslog.filter.severity),
        Message::SyslogSeverityPicked,
    )
    .width(SEVERITY_WIDTH)
    .text_size(INPUT_TEXT_SIZE);
    let (_, shown) = syslog.shown_as_text();
    let copy = small_button("Copy shown", (shown > 0).then(|| Message::SyslogCopyShown));
    let clear = small_button("Clear", (syslog.kept() > 0).then_some(Message::SyslogClear));
    LayoutRow::with_children([
        input_group(vec![host.into(), search.into()]),
        severity.into(),
        copy,
        clear,
    ])
    .spacing(8)
    .align_y(Alignment::Center)
    .wrap()
    .vertical_spacing(8)
    .into()
}

fn messages_view(syslog: &SyslogState) -> Element<'_, Message> {
    if syslog.kept() == 0 {
        return match &syslog.status {
            Status::Listening { .. } => empty_state(
                "Waiting for messages",
                "Nothing has arrived yet. Point a device at one of the addresses above.",
            ),
            _ => empty_state(
                "Not listening",
                "Press Start listening. Nothing is listened for until you do.",
            ),
        };
    }
    let rows = syslog.shown(SHOWN_MESSAGES);
    if rows.is_empty() {
        return empty_state("No match", "No message fits the filters.");
    }
    let narrowed = if syslog.filter.active() {
        ", filtered"
    } else {
        ""
    };
    let count = format!(
        "{} shown of {} kept ({} received{narrowed})",
        rows.len(),
        syslog.kept(),
        syslog.received
    );
    let list = rows
        .into_iter()
        .fold(column![].spacing(2), |list, message| {
            list.push(message_row(message))
        });
    column![text(count).size(12).color(MUTED), scrolling(list)]
        .spacing(6)
        .height(Length::Fill)
        .into()
}

fn message_row(message: &SyslogMessage) -> Element<'_, Message> {
    let host = message
        .hostname
        .clone()
        .unwrap_or_else(|| message.source.to_string());
    row![
        text(format_unix_time_utc(message.received_at_unix))
            .size(ROW_TEXT_SIZE)
            .color(MUTED)
            .width(TIME_WIDTH),
        severity_label(message.severity),
        text(host).size(ROW_TEXT_SIZE).width(HOST_WIDTH),
        text(message.app.clone().unwrap_or_default())
            .size(ROW_TEXT_SIZE)
            .color(MUTED)
            .width(APP_WIDTH),
        text(message.text.as_str())
            .size(ROW_TEXT_SIZE)
            .font(Font::MONOSPACE)
            .width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Start)
    .into()
}

fn severity_label(severity: Option<Severity>) -> Element<'static, Message> {
    let (label, color) = match severity {
        Some(Severity::Emergency | Severity::Alert | Severity::Critical | Severity::Error) => {
            (severity.map_or_else(String::new, |s| s.to_string()), BAD)
        }
        Some(Severity::Warning) => ("Warning".to_owned(), WARN),
        Some(other) => (other.to_string(), MUTED),
        None => (String::new(), MUTED),
    };
    text(label)
        .size(ROW_TEXT_SIZE)
        .font(BOLD)
        .color(color)
        .width(SEVERITY_COLUMN_WIDTH)
        .into()
}
