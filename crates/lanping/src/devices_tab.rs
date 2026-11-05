//! The pages of the Devices tab: the scan results, and what the network announces.

use iced::Element;

use crate::app::Message;
use crate::ui::page_strip;

/// The pages of the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DevicesPage {
    #[default]
    Devices,
    Services,
}

impl DevicesPage {
    const ALL: [Self; 2] = [Self::Devices, Self::Services];

    fn title(self) -> &'static str {
        match self {
            Self::Devices => "Devices",
            Self::Services => "Services",
        }
    }
}

/// The buttons that switch between the pages, with `selected` highlighted.
pub fn devices_strip<'a>(selected: DevicesPage) -> Element<'a, Message> {
    page_strip(
        &DevicesPage::ALL,
        selected,
        DevicesPage::title,
        Message::DevicesPagePicked,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_its_own_title() {
        let titles: Vec<&str> = DevicesPage::ALL.iter().map(|page| page.title()).collect();
        assert_eq!(titles, ["Devices", "Services"]);
    }
}
