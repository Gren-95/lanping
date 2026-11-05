//! Drawing of the SNMP tab.

use iced::font::Weight;
use iced::widget::{
    Row as LayoutRow, button, checkbox, column, container, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Font, Length};
use lanping_core::snmp_interfaces::{HISTORY_LENGTH, Interface};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, lucide, with_tooltip};
use crate::results::pill;
use crate::snmp_tab::{SnmpState, Status, format_rate, format_uptime, utilization_percent};
use crate::sparkline::{Bar, sparkline, width_of};
use crate::table_grid::{cell_width, grid_line};
use crate::theme::{self, ACCENT, BAD, Edge, GOOD, MUTED, WARN};
use crate::view::input_group;

const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};
const PANEL_PADDING: f32 = 16.0;
const SECTION_SPACING: f32 = 14.0;
const SCROLLBAR_GAP: f32 = 12.0;
const TEXT_SIZE: u32 = 13;
const COMMUNITY_WIDTH: f32 = 190.0;
const UTILIZATION_WARN_PERCENT: f64 = 50.0;
const UTILIZATION_BAD_PERCENT: f64 = 80.0;
/// Smallest scale of an interface's graph, so a trickle does not fill the whole bar.
const MIN_GRAPH_SCALE_BPS: f64 = 1e6;
const NO_VALUE: &str = "-";

const COLUMNS: [(&str, f32); 6] = [
    ("Interface", 230.0),
    ("Status", 70.0),
    ("Speed", 90.0),
    ("In", 110.0),
    ("Out", 110.0),
    ("Used", 70.0),
];

pub fn snmp_panel(app: &LanPing) -> Element<'_, Message> {
    let snmp = &app.snmp;
    let body = column![controls(app), summary(snmp), content(snmp)].spacing(SECTION_SPACING);
    let page = scrollable(container(body).padding(iced::Padding::ZERO.right(SCROLLBAR_GAP)))
        .width(Length::Fill)
        .height(Length::Fill);
    container(container(page).padding(PANEL_PADDING).height(Length::Fill))
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn controls(app: &LanPing) -> Element<'_, Message> {
    let snmp = &app.snmp;
    let running = snmp.running();
    let hint = snmp.gateway_hint.map_or_else(
        || "Router or switch address, with :port if it is not 161".to_owned(),
        |ip| format!("Router or switch address, such as {ip} (with :port if it is not 161)"),
    );
    let mut target = text_input(&hint, &snmp.target)
        .padding(theme::CONTROL_PADDING)
        .size(TEXT_SIZE)
        .width(Length::Fill)
        .style(theme::input_at(Edge::First));
    let mut community = text_input("SNMP community", &app.settings.snmp_community)
        .secure(true)
        .padding(theme::CONTROL_PADDING)
        .size(TEXT_SIZE)
        .width(COMMUNITY_WIDTH)
        .style(theme::input_at(Edge::Middle));
    if !running {
        target = target
            .on_input(Message::SnmpTargetChanged)
            .on_submit(Message::SnmpStart);
        community = community
            .on_input(Message::SnmpCommunityChanged)
            .on_submit(Message::SnmpStart);
    }
    let action = if running {
        with_tooltip(
            button(lucide::square().size(ICON_SIZE + 2.0))
                .on_press(Message::SnmpStop)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::danger_button)),
            "Stop",
        )
    } else {
        with_tooltip(
            button(lucide::play().size(ICON_SIZE + 2.0))
                .on_press(Message::SnmpStart)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::primary_button)),
            "Start",
        )
    };
    column![
        input_group(vec![target.into(), community.into(), action]),
        text("Read-only SNMP v2c. The community is like a password for the device; LanPing never guesses it. It shows traffic per interface, not per device.")
            .size(12)
            .color(MUTED),
    ]
    .spacing(6)
    .into()
}

fn summary(snmp: &SnmpState) -> Element<'_, Message> {
    let mut items: Vec<Element<'_, Message>> = Vec::new();
    match &snmp.status {
        Status::Connecting => items.push(pill("Connecting", WARN)),
        Status::Running => {
            items.push(pill("Live", GOOD));
            if let Some(uptime) = snmp.tracker.uptime_seconds() {
                items.push(pill(format!("Up {}", format_uptime(uptime)), ACCENT));
            }
            if snmp.failures > 0 {
                items.push(pill("Missed a reading", WARN));
            }
        }
        Status::Idle | Status::Failed(_) => {}
    }
    items.push(space::horizontal().into());
    if !snmp.interfaces.is_empty() {
        items.push(
            checkbox(snmp.show_all)
                .label("Show all interfaces")
                .size(16)
                .text_size(TEXT_SIZE)
                .on_toggle(Message::SnmpShowAllToggled)
                .into(),
        );
    }
    LayoutRow::with_children(items)
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

fn content(snmp: &SnmpState) -> Element<'_, Message> {
    match &snmp.status {
        Status::Idle => note(
            "Enter the address of a router or switch that has SNMP turned on, and its read-only community, then press start.",
        ),
        Status::Connecting => note("Asking the device for its interfaces."),
        Status::Failed(message) => text(message.as_str()).color(BAD).into(),
        Status::Running => interface_table(snmp),
    }
}

fn note<'a>(message: &'static str) -> Element<'a, Message> {
    text(message).size(14).color(MUTED).into()
}

fn graph_width() -> f32 {
    cell_width(width_of(HISTORY_LENGTH))
}

fn interface_table(snmp: &SnmpState) -> Element<'_, Message> {
    let visible = snmp.visible();
    if visible.is_empty() {
        return note("No interface is up. Tick \"Show all interfaces\" to see the others.");
    }
    let mut header: Vec<(f32, Element<'_, Message>)> = COLUMNS
        .iter()
        .map(|(title, width)| {
            (
                cell_width(*width),
                text(*title).size(13).color(MUTED).into(),
            )
        })
        .collect();
    header.push((graph_width(), text("Traffic").size(13).color(MUTED).into()));
    let lines = visible.into_iter().enumerate().fold(
        column![grid_line(header)],
        |list, (index, interface)| {
            list.push(
                container(grid_line(cells(snmp, interface))).style(theme::stripe(index % 2 == 1)),
            )
        },
    );
    scrollable(container(lines).padding(iced::Padding::ZERO.bottom(SCROLLBAR_GAP)))
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default(),
        ))
        .width(Length::Fill)
        .into()
}

fn cells<'a>(traffic: &'a SnmpState, interface: &'a Interface) -> Vec<(f32, Element<'a, Message>)> {
    let rate = snmp.tracker.rate(interface.index);
    let name: Element<'a, Message> = match &interface.alias {
        Some(alias) => column![
            text(interface.name.as_str()).font(BOLD),
            text(alias.as_str()).size(11).color(MUTED),
        ]
        .into(),
        None => text(interface.name.as_str()).font(BOLD).into(),
    };
    let status = if interface.up {
        pill("Up", GOOD)
    } else {
        pill("Down", MUTED)
    };
    let speed: Element<'a, Message> = match interface.speed_mbps {
        Some(mbps) => text(format_rate(mbps as f64 * 1e6)).into(),
        None => text(NO_VALUE).color(MUTED).into(),
    };
    let rate_text = |bps: Option<f64>| -> Element<'a, Message> {
        match bps {
            Some(bps) => text(format_rate(bps)).into(),
            None => text(NO_VALUE).color(MUTED).into(),
        }
    };
    let used: Element<'a, Message> = match rate
        .and_then(|rate| utilization_percent(rate.in_bps.max(rate.out_bps), interface.speed_mbps))
    {
        Some(percent) => pill(format!("{percent:.0}%"), utilization_color(percent)),
        None => text(NO_VALUE).color(MUTED).into(),
    };
    let values: [Element<'a, Message>; 6] = [
        name,
        status,
        speed,
        rate_text(rate.map(|rate| rate.in_bps)),
        rate_text(rate.map(|rate| rate.out_bps)),
        used,
    ];
    let mut cells: Vec<(f32, Element<'a, Message>)> = COLUMNS
        .iter()
        .map(|(_, width)| cell_width(*width))
        .zip(values)
        .collect();
    cells.push((graph_width(), traffic_graph(snmp, interface.index)));
    cells
}

fn utilization_color(percent: f64) -> iced::Color {
    if percent >= UTILIZATION_BAD_PERCENT {
        BAD
    } else if percent >= UTILIZATION_WARN_PERCENT {
        WARN
    } else {
        GOOD
    }
}

/// One bar per reading, as tall as the traffic in and out together.
fn traffic_graph(snmp: &SnmpState, index: u32) -> Element<'_, Message> {
    let history = snmp.tracker.history(index);
    let scale = history
        .iter()
        .map(|rate| rate.total_bps())
        .fold(MIN_GRAPH_SCALE_BPS, f64::max);
    sparkline(history.iter().map(|rate| Bar {
        fraction: Some((rate.total_bps() / scale) as f32),
        color: ACCENT,
    }))
}
