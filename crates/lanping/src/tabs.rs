//! The pages the window can show, and the bar that switches between them.

use iced::widget::{Row as LayoutRow, Text, button, row, scrollable, text};
use iced::{Alignment, Element, Length, Padding};
use iced_fonts::lucide;
use serde::{Deserialize, Serialize};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, with_tooltip};
use crate::theme;

const TAB_SPACING: f32 = 8.0;
const TAB_PADDING: [f32; 2] = [7.0, 16.0];
const GEAR_PADDING: [f32; 2] = [7.0, 12.0];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Tab {
    #[default]
    Scan,
    Wifi,
    Trace,
    Latency,
    Speed,
    Dns,
    Tls,
    Snmp,
    Subnet,
    Ntp,
    Upnp,
    Services,
    Mtu,
    Network,
    Web,
    /// Not part of the bar's list: it has its own button on the right and cannot be hidden.
    Settings,
}

impl Tab {
    /// The tabs that can be shown, hidden and reordered, in their default order.
    pub const CONFIGURABLE: [Tab; 15] = [
        Tab::Scan,
        Tab::Wifi,
        Tab::Trace,
        Tab::Latency,
        Tab::Speed,
        Tab::Dns,
        Tab::Tls,
        Tab::Snmp,
        Tab::Subnet,
        Tab::Ntp,
        Tab::Upnp,
        Tab::Services,
        Tab::Mtu,
        Tab::Network,
        Tab::Web,
    ];

    /// Whether the tab shows until the user turns it on or off. The six main tabs do; every
    /// other tool is one tick away in Settings, so a new tool starts hidden.
    pub fn shown_by_default(self) -> bool {
        matches!(
            self,
            Self::Scan | Self::Wifi | Self::Trace | Self::Latency | Self::Speed | Self::Dns
        )
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Scan => "Devices",
            Self::Wifi => "Wi-Fi",
            Self::Trace => "Trace",
            Self::Latency => "Latency",
            Self::Speed => "Speed",
            Self::Dns => "DNS",
            Self::Tls => "TLS",
            Self::Snmp => "SNMP",
            Self::Subnet => "Subnet",
            Self::Ntp => "Clock",
            Self::Upnp => "UPnP",
            Self::Services => "Services",
            Self::Mtu => "MTU",
            Self::Network => "My network",
            Self::Web => "Web",
            Self::Settings => "Settings",
        }
    }

    pub fn icon(self) -> Text<'static> {
        match self {
            Self::Scan => lucide::scan_search(),
            Self::Wifi => lucide::wifi(),
            Self::Trace => lucide::activity(),
            Self::Latency => lucide::gauge(),
            Self::Speed => lucide::zap(),
            Self::Dns => lucide::globe(),
            Self::Tls => lucide::shield_check(),
            Self::Snmp => lucide::arrow_down_up(),
            Self::Subnet => lucide::calculator(),
            Self::Ntp => lucide::alarm_clock_check(),
            Self::Upnp => lucide::network(),
            Self::Services => lucide::radio(),
            Self::Mtu => lucide::ruler(),
            Self::Network => lucide::house(),
            Self::Web => lucide::globe_lock(),
            Self::Settings => lucide::settings(),
        }
    }
}

/// The row of tab buttons above the page. A window too narrow for the row scrolls it sideways.
pub fn tab_bar(app: &LanPing) -> Element<'_, Message> {
    let tabs = LayoutRow::with_children(app.settings.visible_tabs().into_iter().map(|tab| {
        button(
            row![tab.icon().size(ICON_SIZE), text(tab.title()).size(14)]
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
