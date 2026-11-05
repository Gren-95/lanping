//! The host list: a scrolling table, or stacked cards when the card view is on.

use iced::widget::{
    Row as LayoutRow, button, column, container, mouse_area, responsive, row, scrollable, space,
    text, tooltip,
};
use iced::{Alignment, Element, Font, Length};
use lanping_core::Host;
use lanping_core::device_type::{DeviceType, guess_device_type};
use lanping_core::port_risk::port_hint;

use crate::app::{LanPing, Message};
use crate::host_filter::RowStatus;
use crate::host_rows::Row;
use crate::icons::{ICON_SIZE, copy_button, icon_button, lucide, with_tooltip};
use crate::settings::Column;
use crate::theme::{
    self, ACCENT, BAD, BOLD, Edge, GOOD, MUTED, NO_VALUE, SCROLLBAR_GAP, WARN, latency_color,
};

const CARD_SPACING: f32 = 10.0;
const CARD_PADDING: f32 = 14.0;
const CELL_PADDING: f32 = 8.0;
const CELL_SIDE_PADDING: f32 = 12.0;

pub fn results<'a>(app: &'a LanPing) -> Element<'a, Message> {
    let rows = app.visible_rows();
    if rows.is_empty() {
        return empty_state(app);
    }
    let columns = app.settings.visible_columns();
    if app.settings.card_view {
        cards(rows, &columns)
    } else {
        host_table(app, rows, &columns)
    }
}

fn empty_state(app: &LanPing) -> Element<'_, Message> {
    let (title, detail) = if app.scanning {
        ("Listening for devices", "Hosts appear here as they answer.")
    } else if app.hosts.is_empty() && app.diff.gone.is_empty() {
        (
            "No scan yet",
            "Enter a network or range above and press Scan to see what is connected.",
        )
    } else {
        (
            "Nothing matches",
            "Adjust or clear the filters to see every host again.",
        )
    };
    container(
        column![
            text(title).size(20).font(BOLD),
            text(detail).size(14).color(MUTED),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .center(Length::Fill)
    .padding(24)
    .into()
}

fn cards<'a>(rows: Vec<Row<'a>>, columns: &[Column]) -> Element<'a, Message> {
    let list = rows.into_iter().fold(
        column![].spacing(CARD_SPACING).width(Length::Fill),
        |list, row| list.push(host_card(row, columns)),
    );
    scrollable(container(list).padding(iced::Padding::ZERO.right(12.0)))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn host_card<'a>(row_data: Row<'a>, columns: &[Column]) -> Element<'a, Message> {
    let host = row_data.host;
    let shows = |column: Column| columns.contains(&column);

    let mut top = row![text(host.ip.to_string()).size(18).font(BOLD)]
        .spacing(8)
        .align_y(Alignment::Center);
    if let Some(badge) = status_badge(row_data.status) {
        top = top.push(badge);
    }
    top = top.push(space::horizontal());
    if shows(Column::Latency) && row_data.status != RowStatus::Gone {
        top = top.push(latency_pill(host));
    }
    top = top.push(open_button(host));

    let mut details = column![top].spacing(4);
    if shows(Column::Hostname) {
        details = details.push(name_text(row_data).size(14));
    }
    let mut identity: Vec<Element<'a, Message>> = Vec::new();
    if let (true, Some(vendor)) = (shows(Column::Vendor), &host.vendor) {
        identity.push(text(vendor.clone()).size(12).color(MUTED).into());
    }
    if shows(Column::Mac) && host.mac.is_some() {
        identity.push(mac_cell(host).size(12).into());
    }
    if !identity.is_empty() {
        details = details.push(
            LayoutRow::with_children(identity)
                .spacing(10)
                .align_y(Alignment::Center)
                .wrap()
                .vertical_spacing(4),
        );
    }
    if shows(Column::Ports) && !host.open_ports.is_empty() {
        details = details.push(port_pills(host));
    }

    clickable_row(
        container(details)
            .padding(CARD_PADDING)
            .width(Length::Fill)
            .style(theme::card),
        host,
    )
}

/// Opens the device detail dialog when the row is clicked; buttons inside keep their own clicks.
fn clickable_row<'a>(row: impl Into<Element<'a, Message>>, host: &Host) -> Element<'a, Message> {
    mouse_area(row)
        .on_press(Message::ShowHostDetails(host.device_key()))
        .interaction(iced::mouse::Interaction::Pointer)
        .into()
}

/// A header plus striped lines on a shared column grid, so every other row is tinted.
/// Columns have fixed widths; spare room is shared by the wide text columns, and a
/// window too narrow for the grid scrolls sideways.
fn host_table<'a>(app: &'a LanPing, rows: Vec<Row<'a>>, shown: &[Column]) -> Element<'a, Message> {
    let shown = shown.to_vec();
    responsive(move |size| {
        let widths = column_widths(&shown, size.width);
        let header =
            LayoutRow::with_children(shown.iter().zip(&widths).map(|(&column, &width)| {
                let cell = container(column_title(app, column))
                    .width(Length::Fixed(width))
                    .padding([CELL_PADDING, CELL_SIDE_PADDING]);
                mouse_area(cell)
                    .on_right_press(Message::ShowColumnMenu(column))
                    .into()
            }))
            .align_y(Alignment::Center);

        let lines = rows
            .iter()
            .enumerate()
            .fold(column![], |list, (index, &row_data)| {
                let cells: Vec<(f32, Element<'a, Message>)> = shown
                    .iter()
                    .zip(&widths)
                    .map(|(&column, &width)| (width, cell(column, row_data)))
                    .collect();
                list.push(clickable_row(
                    container(grid_line(cells)).style(theme::stripe(index % 2 == 1)),
                    row_data.host,
                ))
            });

        scrollable(
            container(column![header, lines]).padding(
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
    })
    .into()
}

/// Narrowest width each column needs for its usual content.
fn base_width(column: Column) -> f32 {
    match column {
        Column::Address => 190.0,
        Column::Latency => 110.0,
        Column::Hostname => 320.0,
        Column::Mac => 170.0,
        Column::Vendor => 200.0,
        Column::Ports => 220.0,
        Column::DeviceType => 150.0,
        Column::Actions => 80.0,
    }
}

/// Columns that take their share of any room left over.
fn grows(column: Column) -> bool {
    matches!(column, Column::Hostname | Column::Vendor | Column::Ports)
}

fn column_widths(shown: &[Column], available: f32) -> Vec<f32> {
    let base_total: f32 = shown.iter().map(|&column| base_width(column)).sum();
    let growing = shown.iter().filter(|&&column| grows(column)).count();
    let spare = (available - SCROLLBAR_GAP - base_total).max(0.0);
    let share = if growing == 0 {
        0.0
    } else {
        spare / growing as f32
    };
    shown
        .iter()
        .map(|&column| base_width(column) + if grows(column) { share } else { 0.0 })
        .collect()
}

fn grid_line<'a>(cells: Vec<(f32, Element<'a, Message>)>) -> LayoutRow<'a, Message> {
    LayoutRow::with_children(cells.into_iter().map(|(width, cell)| {
        container(cell)
            .width(Length::Fixed(width))
            .padding([CELL_PADDING, CELL_SIDE_PADDING])
            .into()
    }))
    .align_y(Alignment::Center)
}

fn cell<'a>(column: Column, row_data: Row<'a>) -> Element<'a, Message> {
    let host = row_data.host;
    match column {
        Column::Address => {
            let mut address = row![text(host.ip.to_string()).font(BOLD)]
                .spacing(8)
                .align_y(Alignment::Center);
            if let Some(badge) = status_badge(row_data.status) {
                address = address.push(badge);
            }
            address
                .push(copy_button(host.ip.to_string(), "IP address"))
                .into()
        }
        Column::Latency if row_data.status == RowStatus::Gone => text(NO_VALUE).color(MUTED).into(),
        Column::Latency => latency_pill(host),
        Column::Hostname => name_text(row_data).into(),
        Column::Mac => match &host.mac {
            Some(mac) => row![mac_cell(host), copy_button(mac.clone(), "MAC address")]
                .spacing(8)
                .align_y(Alignment::Center)
                .into(),
            None => mac_cell(host).into(),
        },
        Column::Vendor => match &host.vendor {
            Some(vendor) => text(vendor.clone()).into(),
            None => text(NO_VALUE).color(MUTED).into(),
        },
        Column::Ports => port_pills(host),
        Column::DeviceType => device_type_cell(host),
        Column::Actions => open_button(host),
    }
}

/// The user's label in the accent color, else the network hostname.
fn name_text<'a>(row_data: Row<'a>) -> iced::widget::Text<'a> {
    match (row_data.label, row_data.host.display_name()) {
        (Some(label), _) => text(label).font(BOLD).color(ACCENT),
        (None, Some(hostname)) => text(hostname.to_owned()),
        (None, None) => text("Unknown device").color(MUTED),
    }
}

fn status_badge<'a>(status: RowStatus) -> Option<Element<'a, Message>> {
    let (label, tint) = match status {
        RowStatus::Unchanged => return None,
        RowStatus::New => ("new", GOOD),
        RowStatus::Changed => ("changed", WARN),
        RowStatus::Gone => ("gone", BAD),
    };
    Some(pill(label, tint))
}

fn open_button<'a>(host: &Host) -> Element<'a, Message> {
    with_tooltip(
        icon_button(
            lucide::external_link().size(ICON_SIZE),
            Some(Message::ShowHostActions(host.device_key())),
            Edge::Alone,
        ),
        "Open with",
    )
}

/// The guessed type as an icon and a word; the tooltip says it is a guess.
fn device_type_cell<'a>(host: &Host) -> Element<'a, Message> {
    let device_type = guess_device_type(host);
    with_tooltip(
        row![
            device_type_icon(device_type).size(ICON_SIZE).color(MUTED),
            text(device_type.to_string()).size(13),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        "Best guess from the name, vendor and open ports",
    )
}

pub fn device_type_icon(device_type: DeviceType) -> iced::widget::Text<'static> {
    match device_type {
        DeviceType::Network => lucide::router(),
        DeviceType::Computer => lucide::monitor(),
        DeviceType::Storage => lucide::hard_drive(),
        DeviceType::Phone => lucide::smartphone(),
        DeviceType::Printer => lucide::printer(),
        DeviceType::Media => lucide::tv(),
        DeviceType::Camera => lucide::camera(),
        DeviceType::SmartHome => lucide::house(),
        DeviceType::Unknown => lucide::circle_help(),
    }
}

fn mac_cell<'a>(host: &Host) -> iced::widget::Text<'a> {
    text(host.mac.clone().unwrap_or_else(|| NO_VALUE.to_owned()))
        .font(Font::MONOSPACE)
        .size(13)
        .color(MUTED)
}

/// Column title: click to sort when the column has an order.
fn column_title<'a>(app: &LanPing, column: Column) -> Element<'a, Message> {
    match column.sort_key() {
        Some(key) => sort_header(app, column.title(), key),
        None => text(column.title()).size(13).color(MUTED).into(),
    }
}

fn sort_header<'a>(
    app: &LanPing,
    label: &str,
    key: crate::host_order::SortKey,
) -> Element<'a, Message> {
    let marker = match (app.sort_key == key, app.sort_ascending) {
        (false, _) => "",
        (true, true) => "  ^",
        (true, false) => "  v",
    };
    button(text(format!("{label}{marker}")).size(13))
        .on_press(Message::SortBy(key))
        .padding(0)
        .style(theme::header_button)
        .into()
}

pub fn pill<'a>(
    label: impl iced::widget::text::IntoFragment<'a>,
    tint: iced::Color,
) -> Element<'a, Message> {
    container(text(label).size(12))
        .padding([2.0, 10.0])
        .style(theme::chip(tint))
        .into()
}

fn latency_pill<'a>(host: &Host) -> Element<'a, Message> {
    match host.latency {
        Some(latency) => {
            let milliseconds = latency.as_secs_f64() * 1000.0;
            pill(format!("{milliseconds:.1} ms"), latency_color(milliseconds))
        }
        None => pill("ARP only", MUTED),
    }
}

/// One pill per open port, amber when it is worth a look. Hovering shows why, and the banner.
fn port_pills<'a>(host: &Host) -> Element<'a, Message> {
    if host.open_ports.is_empty() {
        return text(NO_VALUE).color(MUTED).into();
    }
    LayoutRow::with_children(host.open_ports.iter().map(|&port| {
        let hint = port_hint(port);
        let port_pill = pill(port.to_string(), if hint.is_some() { WARN } else { ACCENT });
        let details: Vec<&str> = hint.into_iter().chain(host.banner(port)).collect();
        if details.is_empty() {
            return port_pill;
        }
        tooltip(
            port_pill,
            container(text(details.join("\n")).size(12))
                .padding(8)
                .style(theme::card),
            tooltip::Position::Top,
        )
        .into()
    }))
    .spacing(6)
    .wrap()
    .vertical_spacing(6)
    .into()
}
