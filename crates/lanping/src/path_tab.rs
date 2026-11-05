//! The Path tab: probes of the route to a host, as pages.

use iced::Element;

use crate::app::{LanPing, Message};
use crate::latency_tab::latency_page;
use crate::mtu_tab::mtu_page;
use crate::trace_tab::trace_page;
use crate::ui::{page_strip, tabbed_panel};

/// The pages of the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PathPage {
    #[default]
    Trace,
    Latency,
    Mtu,
}

impl PathPage {
    const ALL: [Self; 3] = [Self::Trace, Self::Latency, Self::Mtu];

    fn title(self) -> &'static str {
        match self {
            Self::Trace => "Trace",
            Self::Latency => "Latency",
            Self::Mtu => "MTU",
        }
    }
}

pub fn path_panel(app: &LanPing) -> Element<'_, Message> {
    let strip = page_strip(
        &PathPage::ALL,
        app.path_page,
        PathPage::title,
        Message::PathPagePicked,
    );
    let page = match app.path_page {
        PathPage::Trace => trace_page(app),
        PathPage::Latency => latency_page(app),
        PathPage::Mtu => mtu_page(app),
    };
    tabbed_panel(strip, page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_its_own_title() {
        let titles: Vec<&str> = PathPage::ALL.iter().map(|page| page.title()).collect();
        assert_eq!(titles, ["Trace", "Latency", "MTU"]);
    }
}
