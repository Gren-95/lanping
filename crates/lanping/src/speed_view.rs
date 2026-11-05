//! Drawing of the Speed tab.

use iced::font::Weight;
use iced::widget::{
    Row as LayoutRow, button, column, container, pick_list, row, scrollable, space, text,
    text_input,
};
use iced::{Alignment, Element, Font, Length};
use lanping_core::speedtest::SERVICE_HOST;
use lanping_core::throughput::Direction;

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, lucide, with_tooltip};
use crate::sparkline::{Bar, chart};
use crate::speed_tab::{Run, ServerStatus, Source, SpeedPage, SpeedState, TestLength};
use crate::theme::{self, ACCENT, BAD, Edge, GOOD, MUTED, WARN};
use crate::timestamp::format_unix_utc;
use crate::view::input_group;

const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};
const PANEL_PADDING: f32 = 16.0;
const SECTION_SPACING: f32 = 14.0;
const SCROLLBAR_GAP: f32 = 12.0;
const TEXT_SIZE: u32 = 13;
const CHART_HEIGHT: f32 = 100.0;
const TILE_WIDTH: f32 = 150.0;
const TILE_PADDING: f32 = 12.0;
const LENGTH_PICK_WIDTH: f32 = 130.0;
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
    let page = scrollable(container(body).padding(iced::Padding::ZERO.right(SCROLLBAR_GAP)))
        .width(Length::Fill)
        .height(Length::Fill);
    container(container(page).padding(PANEL_PADDING).height(Length::Fill))
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
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

fn note<'a>(message: impl Into<String>) -> Element<'a, Message> {
    text(message.into()).size(14).color(MUTED).into()
}

fn heading<'a>(title: &'static str) -> Element<'a, Message> {
    text(title).size(16).font(BOLD).into()
}

fn internet_page(speed: &SpeedState) -> Element<'_, Message> {
    column![
        note(format!(
            "Measures against {SERVICE_HOST} over HTTPS. Each test sends real traffic there, and that service can see your IP address."
        )),
        test_buttons(speed, Source::Internet),
        run_view(&speed.run),
    ]
    .spacing(SECTION_SPACING)
    .into()
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

/// The length choice and the download, upload and stop buttons.
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
    let start = |direction: Direction, icon: iced::widget::Text<'static>| {
        button(
            row![icon.size(ICON_SIZE), text(direction.to_string()).size(14)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .on_press_maybe((!running).then_some(Message::SpeedStart(source, direction)))
        .padding([7.0, 16.0])
        .style(theme::primary_button)
    };
    let mut controls: Vec<Element<'_, Message>> = vec![
        length.into(),
        start(Direction::Download, lucide::download()).into(),
        start(Direction::Upload, lucide::upload()).into(),
    ];
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
            place,
            direction,
            samples,
        } => {
            let now = samples
                .last()
                .map_or_else(|| "Starting".to_owned(), |mbps| format!("{mbps:.1} Mbit/s"));
            column![
                LayoutRow::with_children([
                    crate::results::pill(format!("{direction} running"), WARN),
                    text(place.as_str()).size(12).color(MUTED).into(),
                ])
                .spacing(8)
                .align_y(Alignment::Center),
                text(now).size(28).font(BOLD),
                samples_chart(samples),
            ]
            .spacing(8)
            .into()
        }
        Run::Done(record) => {
            let result = &record.result;
            column![
                LayoutRow::with_children([
                    crate::results::pill(format!("{} finished", result.direction), GOOD),
                    text(record.place.as_str()).size(12).color(MUTED).into(),
                    text("Saved to history").size(12).color(MUTED).into(),
                ])
                .spacing(8)
                .align_y(Alignment::Center),
                LayoutRow::with_children([
                    tile("Speed", format!("{:.1} Mbit/s", result.mbps)),
                    tile(
                        "Data moved",
                        format!("{:.0} MB", result.bytes as f64 / BYTES_PER_MEGABYTE)
                    ),
                    tile("Time", format!("{:.1} s", result.seconds)),
                ])
                .spacing(SECTION_SPACING)
                .wrap()
                .vertical_spacing(SECTION_SPACING),
                samples_chart(&result.samples_mbps),
            ]
            .spacing(SECTION_SPACING)
            .into()
        }
    }
}

fn tile<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    container(
        column![
            text(label).size(12).color(MUTED),
            text(value).size(20).font(BOLD),
        ]
        .spacing(2),
    )
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
