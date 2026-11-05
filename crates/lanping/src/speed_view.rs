//! Drawing of the Speed tab.

use iced::widget::{
    Row as LayoutRow, button, column, container, pick_list, row, space, text, text_input,
};
use iced::{Alignment, Element, Length};
use lanping_core::ping::PingSummary;
use lanping_core::speedtest::SERVICE_HOST;

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, lucide, with_tooltip};
use crate::sparkline::{Bar, chart};
use crate::speed_tab::{
    Phase, Report, Run, ServerStatus, Source, SpeedPage, SpeedService, SpeedState, TestKind,
    TestLength,
};
use crate::theme::{self, ACCENT, BAD, BOLD, Edge, GOOD, MUTED, SECTION_SPACING, WARN};
use crate::timestamp::format_unix_utc;
use crate::ui::{note, panel, scrolling};
use crate::view::input_group;

const TEXT_SIZE: u32 = 13;
const CHART_HEIGHT: f32 = 100.0;
const TILE_WIDTH: f32 = 170.0;
const TILE_PADDING: f32 = 12.0;
const LENGTH_PICK_WIDTH: f32 = 130.0;
const KIND_PICK_WIDTH: f32 = 190.0;
const SERVICE_PICK_WIDTH: f32 = 260.0;
const LOCATION_PICK_WIDTH: f32 = 230.0;
const PORT_INPUT_WIDTH: f32 = 110.0;
const ROW_PADDING: [f32; 2] = [8.0, 12.0];
const BYTES_PER_MEGABYTE: f64 = 1_000_000.0;

pub fn speed_panel(app: &LanPing) -> Element<'_, Message> {
    let speed = &app.speed;
    let content = match speed.page {
        SpeedPage::Internet => internet_page(speed),
        SpeedPage::Lan => lan_page(speed),
        SpeedPage::History => history_page(speed),
    };
    let body = column![page_strip(speed.page), content].spacing(SECTION_SPACING);
    let page = scrolling(body);
    panel(page)
}

fn page_strip(selected: SpeedPage) -> Element<'static, Message> {
    LayoutRow::with_children(SpeedPage::ALL.into_iter().map(|page| {
        button(text(page.title()).size(TEXT_SIZE))
            .on_press(Message::SpeedPagePicked(page))
            .padding([5.0, 14.0])
            .style(theme::tab_button(page == selected))
            .into()
    }))
    .spacing(8)
    .into()
}

fn heading<'a>(title: &'static str) -> Element<'a, Message> {
    text(title).size(16).font(BOLD).into()
}

fn internet_page(speed: &SpeedState) -> Element<'_, Message> {
    let service = pick_list(
        SpeedService::ALL,
        Some(speed.service),
        Message::SpeedServicePicked,
    )
    .text_size(TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(SERVICE_PICK_WIDTH)
    .style(theme::pick_at(Edge::Alone));
    let mut page = column![service].spacing(SECTION_SPACING);
    match speed.service {
        SpeedService::Cloudflare => {
            page = page.push(note(format!(
                "Measures against {SERVICE_HOST} over HTTPS. Each test sends real traffic there, and that service can see your IP address."
            )));
        }
        SpeedService::Mlab => {
            page = page
                .push(note(
                    "M-Lab runs open measurement servers around the world (the NDT7 protocol). A test lasts at most 10 seconds, and the server can see your IP address.",
                ))
                .push(mlab_servers(speed));
        }
    }
    page.push(test_buttons(speed, speed.service.source()))
        .push(run_view(&speed.run))
        .into()
}

/// The location and server pickers for M-Lab.
fn mlab_servers(speed: &SpeedState) -> Element<'_, Message> {
    let location = pick_list(
        speed.countries.as_slice(),
        Some(&speed.location),
        Message::SpeedLocationPicked,
    )
    .text_size(TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(LOCATION_PICK_WIDTH)
    .style(theme::pick_at(Edge::First));
    let server = pick_list(
        speed.servers.as_slice(),
        speed.selected_server.as_ref(),
        Message::SpeedServerPicked,
    )
    .placeholder(if speed.locating {
        "Looking for servers"
    } else {
        "No server"
    })
    .text_size(TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(Length::Fill)
    .style(theme::pick_at(Edge::Last));
    let mut section = column![input_group(vec![location.into(), server.into()])].spacing(6);
    if let Some(error) = &speed.locate_error {
        section = section.push(
            row![
                text(error.as_str()).size(13).color(BAD),
                button(text("Search again").size(12))
                    .on_press_maybe((!speed.locating).then_some(Message::SpeedServersSearchAgain))
                    .padding([3.0, 10.0])
                    .style(theme::quiet_button),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
        );
    }
    section.into()
}

fn lan_page(speed: &SpeedState) -> Element<'_, Message> {
    let mut address = text_input(
        "Address of the other computer, with :port if it is not 5252",
        &speed.lan_target,
    )
    .padding(theme::CONTROL_PADDING)
    .size(TEXT_SIZE)
    .width(Length::Fill)
    .style(theme::input_at(Edge::Alone));
    if !speed.running() {
        address = address.on_input(Message::SpeedLanTargetChanged);
    }
    column![
        heading("Test against another computer"),
        note("The other computer has to run LanPing with its test server started below."),
        address,
        test_buttons(speed, Source::Lan),
        run_view(&speed.run),
        server_section(speed),
    ]
    .spacing(SECTION_SPACING)
    .into()
}

/// The length and kind choices, and the start and stop buttons.
fn test_buttons(speed: &SpeedState, source: Source) -> Element<'_, Message> {
    let running = speed.running();
    let length = pick_list(
        TestLength::ALL,
        Some(speed.length),
        Message::SpeedLengthPicked,
    )
    .text_size(TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(LENGTH_PICK_WIDTH)
    .style(theme::pick_at(Edge::Alone));
    let kind = pick_list(TestKind::ALL, Some(speed.kind), Message::SpeedKindPicked)
        .text_size(TEXT_SIZE)
        .padding(theme::CONTROL_PADDING)
        .width(KIND_PICK_WIDTH)
        .style(theme::pick_at(Edge::Alone));
    let start = button(
        row![lucide::play().size(ICON_SIZE), text("Start test").size(14)]
            .spacing(8)
            .align_y(Alignment::Center),
    )
    .on_press_maybe((!running).then_some(Message::SpeedStart(source)))
    .padding([7.0, 16.0])
    .style(theme::primary_button);
    let mut controls: Vec<Element<'_, Message>> = vec![length.into(), kind.into(), start.into()];
    if running {
        controls.push(with_tooltip(
            button(lucide::square().size(ICON_SIZE + 2.0))
                .on_press(Message::SpeedStop)
                .padding([7.0, 16.0])
                .style(theme::danger_button),
            "Stop the test",
        ));
    }
    LayoutRow::with_children(controls)
        .spacing(8)
        .align_y(Alignment::Center)
        .wrap()
        .vertical_spacing(8)
        .into()
}

fn run_view(run: &Run) -> Element<'_, Message> {
    match run {
        Run::Idle => space::vertical().height(0).into(),
        Run::Failed(message) => text(message.as_str()).color(BAD).into(),
        Run::Running {
            phase,
            samples,
            report,
        } => report_view(report, Some((*phase, samples))),
        Run::Done(report) => report_view(report, None),
    }
}

/// What a test has found: ping, each finished direction, and the step in progress.
fn report_view<'a>(report: &'a Report, live: Option<(Phase, &'a [f64])>) -> Element<'a, Message> {
    let status = match (live, &report.error) {
        (Some((phase, _)), _) => crate::results::pill(phase.to_string(), WARN),
        (None, Some(_)) => crate::results::pill("Stopped early", WARN),
        (None, None) => crate::results::pill("Finished", GOOD),
    };
    let mut header: Vec<Element<'a, Message>> = vec![
        status,
        text(report.place.as_str()).size(12).color(MUTED).into(),
    ];
    if live.is_none() && !report.results.is_empty() {
        header.push(text("Saved to history").size(12).color(MUTED).into());
    }
    let mut tiles: Vec<Element<'a, Message>> = Vec::new();
    if let Some(ping) = &report.ping {
        tiles.push(ping_tile(ping));
    }
    for result in &report.results {
        tiles.push(tile(
            &result.direction.to_string(),
            format!("{:.1} Mbit/s", result.mbps),
            Some(format!(
                "{:.0} MB in {:.1} s",
                result.bytes as f64 / BYTES_PER_MEGABYTE,
                result.seconds
            )),
        ));
    }
    let mut content = column![
        LayoutRow::with_children(header)
            .spacing(8)
            .align_y(Alignment::Center),
    ]
    .spacing(SECTION_SPACING);
    if !tiles.is_empty() {
        content = content.push(
            LayoutRow::with_children(tiles)
                .spacing(SECTION_SPACING)
                .wrap()
                .vertical_spacing(SECTION_SPACING),
        );
    }
    if let Some(message) = &report.ping_error {
        content = content.push(
            text(format!("Ping failed: {message}"))
                .size(13)
                .color(MUTED),
        );
    }
    if let Some((phase, samples)) = live
        && phase != Phase::Ping
    {
        let now = samples
            .last()
            .map_or_else(|| "Starting".to_owned(), |mbps| format!("{mbps:.1} Mbit/s"));
        content = content.push(text(now).size(28).font(BOLD));
        content = content.push(samples_chart(samples));
    }
    for result in &report.results {
        content = content.push(
            column![
                text(result.direction.to_string()).size(13).color(MUTED),
                samples_chart(&result.samples_mbps),
            ]
            .spacing(4),
        );
    }
    if let Some(message) = &report.error {
        content = content.push(text(message.as_str()).color(BAD));
    }
    content.into()
}

fn ping_tile<'a>(ping: &PingSummary) -> Element<'a, Message> {
    let detail = match (ping.jitter_ms, ping.answered == ping.probes) {
        (Some(jitter), true) => Some(format!("jitter {jitter:.1} ms")),
        (Some(jitter), false) => Some(format!(
            "jitter {jitter:.1} ms, {} of {} answered",
            ping.answered, ping.probes
        )),
        (None, _) => Some(format!("{} of {} answered", ping.answered, ping.probes)),
    };
    tile("Ping", ping.describe(), detail)
}

fn tile<'a>(label: &str, value: String, detail: Option<String>) -> Element<'a, Message> {
    let mut lines = column![
        text(label.to_owned()).size(12).color(MUTED),
        text(value).size(20).font(BOLD),
    ]
    .spacing(2);
    if let Some(detail) = detail {
        lines = lines.push(text(detail).size(11).color(MUTED));
    }
    container(lines)
        .padding(TILE_PADDING)
        .width(TILE_WIDTH)
        .style(theme::card)
        .into()
}

/// One bar per second, as tall as that second's speed against the fastest second.
fn samples_chart<'a>(samples: &[f64]) -> Element<'a, Message> {
    if samples.is_empty() {
        return space::vertical().height(0).into();
    }
    let fastest = samples.iter().copied().fold(1.0, f64::max);
    let bars = samples.iter().map(|mbps| Bar {
        fraction: Some((mbps / fastest) as f32),
        color: ACCENT,
    });
    column![
        container(chart(bars, CHART_HEIGHT))
            .style(theme::card)
            .padding(TILE_PADDING),
        text(format!(
            "One bar per second, full height is {fastest:.0} Mbit/s"
        ))
        .size(12)
        .color(MUTED),
    ]
    .spacing(6)
    .into()
}

fn server_section(speed: &SpeedState) -> Element<'_, Message> {
    let listening = matches!(
        speed.server,
        ServerStatus::Listening { .. } | ServerStatus::Starting
    );
    let mut port = text_input("5252", &speed.server_port)
        .padding(theme::CONTROL_PADDING)
        .size(TEXT_SIZE)
        .width(PORT_INPUT_WIDTH)
        .style(theme::input_at(Edge::First));
    if !listening {
        port = port.on_input(Message::SpeedServerPortChanged);
    }
    let action = if listening {
        with_tooltip(
            button(lucide::square().size(ICON_SIZE + 2.0))
                .on_press(Message::SpeedServerStop)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::danger_button)),
            "Stop the test server",
        )
    } else {
        with_tooltip(
            button(lucide::play().size(ICON_SIZE + 2.0))
                .on_press(Message::SpeedServerStart)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::primary_button)),
            "Start the test server",
        )
    };
    let status: Element<'_, Message> = match &speed.server {
        ServerStatus::Stopped => note(
            "Not running. Anyone on the network can run a test against this computer while it is.",
        ),
        ServerStatus::Starting => note("Starting."),
        ServerStatus::Failed(message) => text(message.as_str()).color(BAD).into(),
        ServerStatus::Listening { port, addresses } => {
            let places: Vec<String> = addresses
                .iter()
                .map(|address| format!("{address}:{port}"))
                .collect();
            let places = if places.is_empty() {
                format!("port {port}")
            } else {
                places.join(", ")
            };
            text(format!("Listening on {places}")).color(GOOD).into()
        }
    };
    let served = speed
        .served
        .iter()
        .fold(column![].spacing(4), |list, test| {
            list.push(
                text(format!(
                    "{} {}: {:.0} MB",
                    test.peer,
                    test.direction.to_string().to_lowercase(),
                    test.bytes as f64 / BYTES_PER_MEGABYTE
                ))
                .size(12)
                .color(MUTED),
            )
        });
    column![
        heading("Serve tests from this computer"),
        input_group(vec![port.into(), action]),
        status,
        served,
    ]
    .spacing(8)
    .into()
}

fn history_page(speed: &SpeedState) -> Element<'_, Message> {
    if speed.history.is_empty() {
        return note("Finished speed tests and latency runs are saved here.");
    }
    speed
        .history
        .iter()
        .enumerate()
        .fold(column![], |list, (index, (_, measurement))| {
            let line = row![
                column![
                    text(measurement.title()).font(BOLD),
                    text(measurement.summary()).size(12).color(MUTED),
                ]
                .spacing(2)
                .width(Length::Fill),
                text(format_unix_utc(measurement.finished_at_unix()))
                    .size(12)
                    .color(MUTED),
            ]
            .align_y(Alignment::Center);
            list.push(
                container(line)
                    .padding(ROW_PADDING)
                    .width(Length::Fill)
                    .style(theme::stripe(index % 2 == 1)),
            )
        })
        .into()
}
