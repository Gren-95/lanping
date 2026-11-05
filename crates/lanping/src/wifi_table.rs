//! The sortable access point table on the Wi-Fi tab.

use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Color, Element, Font, Length};
use lanping_core::vendor::lookup_vendor;
use lanping_core::wifi::{AccessPoint, SignalHistory};

use crate::app::Message;
use crate::icons::copy_button;
use crate::results::pill;
use crate::sparkline::{Bar, sparkline, width_of};
use crate::table_grid::{cell_width, grid_line};
use crate::theme::{self, BAD, GOOD, MUTED, WARN};
use crate::wifi_list::WifiSortKey;

const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};
/// Keeps the scrollbar clear of the last row.
const SCROLLBAR_GAP: f32 = 12.0;
const STRONG_SIGNAL: u8 = 67;
const FAIR_SIGNAL: u8 = 34;
const NO_VALUE: &str = "-";
const TREND_TITLE: &str = "Trend";
/// The trend column sits right after Signal.
const TREND_POSITION: usize = 6;

fn trend_width() -> f32 {
    cell_width(width_of(SignalHistory::LENGTH))
}

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
    history: &SignalHistory,
) -> Element<'a, Message> {
    let mut header_cells: Vec<(f32, Element<'a, Message>)> = WifiSortKey::ALL
        .into_iter()
        .map(|key| (column_width(key), sort_header(key, sort_key, ascending)))
        .collect();
    header_cells.insert(
        TREND_POSITION,
        (trend_width(), text(TREND_TITLE).size(13).into()),
    );
    let header = grid_line(header_cells);
    let lines = rows
        .iter()
        .enumerate()
        .fold(column![header], |list, (index, access_point)| {
            list.push(
                container(grid_line(cells(access_point, history)))
                    .style(theme::stripe(index % 2 == 1)),
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

fn cells<'a>(
    access_point: &'a AccessPoint,
    history: &SignalHistory,
) -> Vec<(f32, Element<'a, Message>)> {
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
    let sized = |key: WifiSortKey, cell: Element<'a, Message>| (column_width(key), cell);
    let mut cells = vec![
        sized(WifiSortKey::Network, name),
        sized(WifiSortKey::Bssid, bssid),
        sized(WifiSortKey::Vendor, vendor),
        sized(
            WifiSortKey::Channel,
            text(access_point.channel.to_string()).into(),
        ),
        sized(
            WifiSortKey::Band,
            text(access_point.band.to_string()).into(),
        ),
        sized(WifiSortKey::Signal, signal),
        sized(WifiSortKey::Security, security),
        sized(WifiSortKey::Rate, rate),
    ];
    cells.insert(
        TREND_POSITION,
        (trend_width(), trend(&history.samples(&access_point.bssid))),
    );
    cells
}

/// One bar per recent scan, as tall as the signal was. A dash marks a scan that missed it.
fn trend<'a>(samples: &[Option<u8>]) -> Element<'a, Message> {
    sparkline(samples.iter().map(|sample| match sample {
        Some(percent) => Bar {
            fraction: Some(f32::from(*percent) / 100.0),
            color: signal_color(*percent),
        },
        None => Bar {
            fraction: None,
            color: MUTED,
        },
    }))
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
