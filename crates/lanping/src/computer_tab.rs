//! The This computer tab: this computer's network facts, its clock and a subnet calculator.

use iced::Element;

use crate::app::{LanPing, Message};
use crate::network_tab::network_page;
use crate::ntp_tab::ntp_page;
use crate::subnet_tab::subnet_page;
use crate::ui::{page_strip, tabbed_panel};

/// The pages of the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ComputerPage {
    #[default]
    Network,
    Clock,
    Subnet,
}

impl ComputerPage {
    const ALL: [Self; 3] = [Self::Network, Self::Clock, Self::Subnet];

    fn title(self) -> &'static str {
        match self {
            Self::Network => "My network",
            Self::Clock => "Clock",
            Self::Subnet => "Subnet",
        }
    }
}

pub fn computer_panel(app: &LanPing) -> Element<'_, Message> {
    let strip = page_strip(
        &ComputerPage::ALL,
        app.computer_page,
        ComputerPage::title,
        Message::ComputerPagePicked,
    );
    let page = match app.computer_page {
        ComputerPage::Network => network_page(app),
        ComputerPage::Clock => ntp_page(app),
        ComputerPage::Subnet => subnet_page(app),
    };
    tabbed_panel(strip, page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_its_own_title() {
        let titles: Vec<&str> = ComputerPage::ALL.iter().map(|page| page.title()).collect();
        assert_eq!(titles, ["My network", "Clock", "Subnet"]);
    }
}
