//! The pages the window can show, and the bar that switches between them.

use iced::widget::{Row as LayoutRow, Text, button, row, scrollable, text};
use iced::{Alignment, Element, Length, Padding};
use iced_fonts::lucide;

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, with_tooltip};
use crate::theme;

const TAB_SPACING: f32 = 8.0;
const TAB_PADDING: [f32; 2] = [7.0, 16.0];
const GEAR_PADDING: [f32; 2] = [7.0, 12.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Scan,
    Wifi,
    Trace,
    Subnet,
    Settings,
}

/// The tabs in the bar, left to right. Settings has its own button on the right.
fn bar_tabs() -> [(Tab, Text<'static>, &'static str); 4] {
    [
        (Tab::Scan, lucide::scan_search(), "Devices"),
        (Tab::Wifi, lucide::wifi(), "Wi-Fi"),
        (Tab::Trace, lucide::activity(), "Trace"),
        (Tab::Subnet, lucide::calculator(), "Subnet"),
    ]
}

/// The row of tab buttons above the page. A window too narrow for the row scrolls it sideways.
pub fn tab_bar(app: &LanPing) -> Element<'_, Message> {
    let tabs = LayoutRow::with_children(bar_tabs().into_iter().map(|(tab, icon, label)| {
        button(
            row![icon.size(ICON_SIZE), text(label).size(14)]
                .spacing(8)
                .align_y(Alignment::Center),
        )
        .on_press(Message::TabSelected(tab))
        .padding(TAB_PADDING)
        .style(theme::tab_button(app.tab == tab))
        .into()
    }))
    .spacing(TAB_SPACING);
    let hidden_bar = scrollable::Scrollbar::new().width(0).scroller_width(0);
    let scrolling_tabs = scrollable(tabs)
        .direction(scrollable::Direction::Horizontal(hidden_bar))
        .width(Length::Fill);
    let settings = with_tooltip(
        button(lucide::settings().size(ICON_SIZE))
            .on_press(Message::TabSelected(Tab::Settings))
            .padding(GEAR_PADDING)
            .style(theme::tab_button(app.tab == Tab::Settings)),
        "Settings",
    );
    row![scrolling_tabs, settings]
        .spacing(TAB_SPACING)
        .padding(Padding::ZERO)
        .align_y(Alignment::Center)
        .into()
}
