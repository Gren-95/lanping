//! The Wi-Fi tab: nearby access points, channel crowding, and a searchable table.

use iced::font::Weight;
use iced::widget::{
    Row as LayoutRow, button, checkbox, column, container, pick_list, row, scrollable, space, text,
    text_input,
};
use iced::{Alignment, Element, Font, Length, Task};
use lanping_core::wifi::{AccessPoint, SignalHistory, WifiError, scan_access_points};

use crate::app::{LanPing, Message, SEARCH_INPUT_ID};
use crate::icons::{ICON_SIZE, icon_button, lucide, with_tooltip};
use crate::results::pill;
use crate::theme::{self, Edge, MUTED, WARN};
use crate::view::input_group;
use crate::wifi_channels::channel_section;
use crate::wifi_list::{
    BandChoice, SecurityChoice, WifiFilter, WifiFilterChange, WifiSortKey, rows_as_text, visible,
};
use crate::wifi_table::access_point_table;

const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};
const PANEL_PADDING: f32 = 16.0;
const SECTION_SPACING: f32 = 14.0;
/// Keeps the scrollbar clear of the content.
const SCROLLBAR_GAP: f32 = 12.0;
/// Windows at least this wide keep the filter controls on one line.
const WIDE_WIDTH: f32 = 820.0;
const CONTROL_TEXT_SIZE: u32 = 13;
const BAND_PICK_WIDTH: f32 = 130.0;
const SECURITY_PICK_WIDTH: f32 = 150.0;

/// The pages the window can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Scan,
    Wifi,
    Trace,
    Settings,
}

/// What the last Wi-Fi scan found, and how it is searched and ordered.
#[derive(Debug, Default)]
pub struct WifiState {
    access_points: Vec<AccessPoint>,
    scanning: bool,
    error: Option<WifiError>,
    has_scanned: bool,
    history: SignalHistory,
    auto_refresh: bool,
    filter: WifiFilter,
    sort_key: WifiSortKey,
    /// Strongest signal first until a header is clicked.
    sort_ascending: bool,
}

impl WifiState {
    pub fn has_scanned(&self) -> bool {
        self.has_scanned
    }

    /// Starts a scan unless one is already running.
    pub fn start(&mut self) -> Task<Message> {
        if self.scanning {
            return Task::none();
        }
        self.scanning = true;
        Task::perform(scan_access_points(), Message::WifiScanned)
    }

    pub fn finish(&mut self, result: Result<Vec<AccessPoint>, WifiError>) {
        self.scanning = false;
        self.has_scanned = true;
        match result {
            Ok(access_points) => {
                self.history.record(&access_points);
                self.access_points = access_points;
                self.error = None;
            }
            Err(error) => {
                self.access_points.clear();
                self.error = Some(error);
            }
        }
    }

    pub fn auto_refresh(&self) -> bool {
        self.auto_refresh
    }

    pub fn set_auto_refresh(&mut self, on: bool) {
        self.auto_refresh = on;
    }

    /// Sorts by `key`; choosing the same key again flips the direction.
    pub fn sort_by(&mut self, key: WifiSortKey) {
        if self.sort_key == key {
            self.sort_ascending = !self.sort_ascending;
        } else {
            self.sort_key = key;
            self.sort_ascending = true;
        }
    }

    pub fn apply_filter(&mut self, change: WifiFilterChange) {
        self.filter.apply(change);
    }

    pub fn clear_filter(&mut self) {
        self.filter.apply(WifiFilterChange::Clear);
    }

    fn rows(&self) -> Vec<&AccessPoint> {
        visible(
            &self.access_points,
            &self.filter,
            self.sort_key,
            self.sort_ascending,
        )
    }

    /// The shown rows as tab-separated text, and how many there are.
    pub fn rows_as_text(&self) -> (String, usize) {
        let rows = self.rows();
        (rows_as_text(&rows), rows.len())
    }
}

/// The row of tab buttons above the page.
pub fn tab_bar(app: &LanPing) -> Element<'_, Message> {
    let tabs = [
        (Tab::Scan, lucide::scan_search(), "Devices"),
        (Tab::Wifi, lucide::wifi(), "Wi-Fi"),
        (Tab::Trace, lucide::activity(), "Trace"),
    ];
    LayoutRow::with_children(tabs.into_iter().map(|(tab, icon, label)| {
        button(
            row![icon.size(ICON_SIZE), text(label).size(14)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .on_press(Message::TabSelected(tab))
        .padding([7.0, 16.0])
        .style(theme::tab_button(app.tab == tab))
        .into()
    }))
    .spacing(8)
    .push(space::horizontal())
    .push(with_tooltip(
        button(lucide::settings().size(ICON_SIZE))
            .on_press(Message::TabSelected(Tab::Settings))
            .padding([7.0, 12.0])
            .style(theme::tab_button(app.tab == Tab::Settings)),
        "Settings",
    ))
    .align_y(Alignment::Center)
    .into()
}

pub fn wifi_panel(app: &LanPing, width: f32) -> Element<'_, Message> {
    let wifi = &app.wifi;
    let rows = wifi.rows();
    let content: Element<'_, Message> = match (&wifi.error, wifi.access_points.is_empty()) {
        (Some(error), _) => empty_state("Cannot scan Wi-Fi", error.to_string()),
        (None, true) if wifi.scanning || !wifi.has_scanned => {
            empty_state("Scanning", "Looking for nearby access points.".to_owned())
        }
        (None, true) => empty_state(
            "No networks found",
            "Check that Wi-Fi is turned on, then refresh.".to_owned(),
        ),
        (None, false) => results(wifi, &rows),
    };
    let page = column![toolbar(wifi, rows.len(), width >= WIDE_WIDTH), content]
        .spacing(SECTION_SPACING)
        .height(Length::Fill);
    container(container(page).padding(PANEL_PADDING).height(Length::Fill))
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn toolbar(wifi: &WifiState, shown: usize, wide: bool) -> Element<'_, Message> {
    let refresh = icon_button(
        lucide::refresh_cw().size(ICON_SIZE),
        (!wifi.scanning).then_some(Message::WifiRefresh),
        Edge::Alone,
    );
    let copy = icon_button(
        lucide::clipboard_copy().size(ICON_SIZE),
        (shown > 0).then_some(Message::WifiCopyRows),
        Edge::Alone,
    );
    let auto = checkbox(wifi.auto_refresh)
        .label("Auto refresh")
        .size(16)
        .text_size(CONTROL_TEXT_SIZE)
        .on_toggle(Message::WifiAutoRefreshToggled);
    let mut items: Vec<Element<'_, Message>> =
        vec![with_tooltip(refresh, "Scan again"), auto.into()];
    if wifi.scanning {
        items.push(pill("Scanning", WARN));
    }
    if !wifi.access_points.is_empty() {
        let total = wifi.access_points.len();
        let label = if shown == total {
            format!("{total} access points")
        } else {
            format!("{shown} of {total} access points")
        };
        items.push(pill(label, theme::ACCENT));
    }
    items.push(space::horizontal().into());
    items.push(with_tooltip(copy, "Copy visible networks"));
    let status = LayoutRow::with_children(items)
        .spacing(8)
        .align_y(Alignment::Center);
    column![status, filter_controls(&wifi.filter, wide)]
        .spacing(8)
        .into()
}

fn filter_controls(filter: &WifiFilter, wide: bool) -> Element<'_, Message> {
    let search_edge = if wide { Edge::First } else { Edge::Alone };
    let search = text_input("Search name, BSSID, vendor or channel", &filter.text)
        .id(SEARCH_INPUT_ID)
        .on_input(|text| Message::WifiFilter(WifiFilterChange::Text(text)))
        .padding(theme::CONTROL_PADDING)
        .size(CONTROL_TEXT_SIZE)
        .width(Length::Fill)
        .style(theme::input_at(search_edge));
    let band_edge = if wide { Edge::Middle } else { Edge::First };
    let band = pick_list(BandChoice::ALL, Some(filter.band), |band| {
        Message::WifiFilter(WifiFilterChange::Band(band))
    })
    .text_size(CONTROL_TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(BAND_PICK_WIDTH)
    .style(theme::pick_at(band_edge));
    let security = pick_list(SecurityChoice::ALL, Some(filter.security), |security| {
        Message::WifiFilter(WifiFilterChange::Security(security))
    })
    .text_size(CONTROL_TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(SECURITY_PICK_WIDTH)
    .style(theme::pick_at(Edge::Middle));
    let clear = with_tooltip(
        icon_button(
            lucide::funnel_x().size(ICON_SIZE),
            (filter.active_count() > 0).then_some(Message::WifiFilter(WifiFilterChange::Clear)),
            Edge::Last,
        ),
        "Clear filters",
    );
    let rest = [band.into(), security.into(), clear];
    if wide {
        input_group(std::iter::once(search.into()).chain(rest).collect())
    } else {
        column![search, input_group(rest.into())].spacing(8).into()
    }
}

fn empty_state<'a>(title: &'static str, detail: String) -> Element<'a, Message> {
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

fn results<'a>(wifi: &'a WifiState, rows: &[&'a AccessPoint]) -> Element<'a, Message> {
    let table: Element<'a, Message> = if rows.is_empty() {
        text("No network matches the search and filters.")
            .size(14)
            .color(MUTED)
            .into()
    } else {
        access_point_table(rows, wifi.sort_key, wifi.sort_ascending, &wifi.history)
    };
    let body = column![channel_section(&wifi.access_points), table].spacing(SECTION_SPACING * 1.5);
    scrollable(container(body).padding(iced::Padding::ZERO.right(SCROLLBAR_GAP)))
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_scan_clears_old_results_and_keeps_the_error() {
        let mut state = WifiState::default();
        state.finish(Err(WifiError::Unsupported));
        assert!(state.has_scanned());
        assert!(state.access_points.is_empty());
        assert_eq!(state.error, Some(WifiError::Unsupported));
    }

    #[test]
    fn a_good_scan_clears_the_previous_error() {
        let mut state = WifiState::default();
        state.finish(Err(WifiError::TimedOut));
        state.finish(Ok(Vec::new()));
        assert_eq!(state.error, None);
    }

    #[test]
    fn a_running_scan_blocks_a_second_one() {
        let mut state = WifiState {
            scanning: true,
            ..WifiState::default()
        };
        let _ = state.start();
        assert!(state.scanning);
    }

    fn scanned_state() -> WifiState {
        let mut state = WifiState::default();
        state.finish(Ok(Vec::new()));
        state
    }

    #[test]
    fn each_good_scan_is_added_to_the_signal_history() {
        let mut state = WifiState::default();
        state.finish(Ok(vec![AccessPoint {
            ssid: String::new(),
            bssid: "02:00:00:00:00:01".to_owned(),
            channel: 6,
            frequency_mhz: 2437,
            band: lanping_core::wifi::Band::Ghz2_4,
            signal_percent: 70,
            security: String::new(),
            rate_mbps: None,
        }]));
        state.finish(Err(WifiError::TimedOut));
        assert_eq!(state.history.samples("02:00:00:00:00:01"), [Some(70)]);
    }

    #[test]
    fn sorting_the_same_key_twice_flips_direction() {
        let mut state = scanned_state();
        state.sort_by(WifiSortKey::Network);
        assert!(state.sort_ascending);
        state.sort_by(WifiSortKey::Network);
        assert!(!state.sort_ascending);
        state.sort_by(WifiSortKey::Channel);
        assert!(state.sort_ascending);
    }

    #[test]
    fn filters_report_when_active_and_clear_again() {
        let mut state = scanned_state();
        assert_eq!(state.filter.active_count(), 0);
        state.apply_filter(WifiFilterChange::Text("home".to_owned()));
        assert_eq!(state.filter.active_count(), 1);
        state.clear_filter();
        assert_eq!(state.filter.active_count(), 0);
    }

    #[test]
    fn copied_text_counts_only_shown_rows() {
        let (text, count) = scanned_state().rows_as_text();
        assert_eq!(count, 0);
        assert_eq!(text.lines().count(), 1);
    }
}
