//! The sortable access point table on the Wi-Fi tab.

use iced::font::Weight;
use iced::widget::{Row as LayoutRow, button, column, container, row, scrollable, text};
use iced::{Alignment, Color, Element, Font, Length};
use lanping_core::vendor::lookup_vendor;
use lanping_core::wifi::AccessPoint;

use crate::app::Message;
use crate::icons::copy_button;
use crate::results::pill;
use crate::theme::{self, BAD, GOOD, MUTED, WARN};
use crate::wifi_list::WifiSortKey;

const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};
const CELL_PADDING: f32 = 8.0;
const CELL_SIDE_PADDING: f32 = 12.0;
/// Keeps the scrollbar clear of the last row.
const SCROLLBAR_GAP: f32 = 12.0;
const STRONG_SIGNAL: u8 = 67;
const FAIR_SIGNAL: u8 = 34;
const NO_VALUE: &str = "-";

fn column_width(key: WifiSortKey) -> f32 {
    match key {
        WifiSortKey::Network => 240.0,
        WifiSortKey::Bssid => 190.0,
        WifiSortKey::Vendor => 200.0,
        WifiSortKey::Channel => 100.0,
        WifiSortKey::Band => 100.0,
        WifiSortKey::Signal => 100.0,
        WifiSortKey::Security => 170.0,
        WifiSortKey::Rate => 120.0,
    }
}

/// A header with a clickable title per column, then striped rows.
pub fn access_point_table<'a>(
    rows: &[&'a AccessPoint],
    sort_key: WifiSortKey,
    ascending: bool,
) -> Element<'a, Message> {
    let header = grid_line(
        WifiSortKey::ALL
            .into_iter()
            .map(|key| (key, sort_header(key, sort_key, ascending)))
            .collect(),
    );
    let lines = rows
        .iter()
        .enumerate()
        .fold(column![header], |list, (index, access_point)| {
            list.push(
                container(grid_line(cells(access_point))).style(theme::stripe(index % 2 == 1)),
            )
        });
    scrollable(container(lines).padding(iced::Padding::ZERO.bottom(SCROLLBAR_GAP)))
        .direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default(),
        ))
        .width(Length::Fill)
        .into()
}

fn sort_header<'a>(
    key: WifiSortKey,
    sorted_by: WifiSortKey,
    ascending: bool,
) -> Element<'a, Message> {
    let marker = match (key == sorted_by, ascending) {
        (false, _) => "",
        (true, true) => "  ^",
        (true, false) => "  v",
    };
    button(text(format!("{}{marker}", key.title())).size(13))
        .on_press(Message::WifiSortBy(key))
        .padding(0)
        .style(theme::header_button)
        .into()
}

fn cells(access_point: &AccessPoint) -> Vec<(WifiSortKey, Element<'_, Message>)> {
    let name: Element<'_, Message> = if access_point.ssid.is_empty() {
        text("(hidden)").color(MUTED).into()
    } else {
        copyable(
            text(access_point.ssid.as_str()).font(BOLD),
            &access_point.ssid,
            "network name",
        )
    };
    let bssid = copyable(
        text(access_point.bssid.as_str()),
        &access_point.bssid,
        "BSSID",
    );
    let vendor: Element<'_, Message> = match lookup_vendor(&access_point.bssid) {
        Some(vendor) => text(vendor).into(),
        None => text(NO_VALUE).color(MUTED).into(),
    };
    let security: Element<'_, Message> = if access_point.security.is_empty() {
        pill("Open", BAD)
    } else {
        text(access_point.security.as_str()).into()
    };
    let rate: Element<'_, Message> = match access_point.rate_mbps {
        Some(rate) => text(format!("{rate} Mbit/s")).into(),
        None => text(NO_VALUE).color(MUTED).into(),
    };
    let signal = pill(
        format!("{}%", access_point.signal_percent),
        signal_color(access_point.signal_percent),
    );
    vec![
        (WifiSortKey::Network, name),
        (WifiSortKey::Bssid, bssid),
        (WifiSortKey::Vendor, vendor),
        (
            WifiSortKey::Channel,
            text(access_point.channel.to_string()).into(),
        ),
        (
            WifiSortKey::Band,
            text(access_point.band.to_string()).into(),
        ),
        (WifiSortKey::Signal, signal),
        (WifiSortKey::Security, security),
        (WifiSortKey::Rate, rate),
    ]
}

/// `content` followed by a button that copies `value`.
fn copyable<'a>(
    content: impl Into<Element<'a, Message>>,
    value: &str,
    label: &'static str,
) -> Element<'a, Message> {
    row![content.into(), copy_button(value.to_owned(), label)]
        .spacing(8)
        .align_y(Alignment::Center)
        .into()
}

fn signal_color(percent: u8) -> Color {
    match percent {
        STRONG_SIGNAL.. => GOOD,
        FAIR_SIGNAL.. => WARN,
        _ => BAD,
    }
}

fn grid_line<'a>(cells: Vec<(WifiSortKey, Element<'a, Message>)>) -> LayoutRow<'a, Message> {
    LayoutRow::with_children(cells.into_iter().map(|(key, cell)| {
        container(cell)
            .width(Length::Fixed(column_width(key)))
            .padding([CELL_PADDING, CELL_SIDE_PADDING])
            .into()
    }))
    .align_y(Alignment::Center)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signal_is_graded_by_strength() {
        assert_eq!(signal_color(90), GOOD);
        assert_eq!(signal_color(50), WARN);
        assert_eq!(signal_color(10), BAD);
    }
}
