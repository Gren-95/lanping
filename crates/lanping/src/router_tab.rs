//! The Router tab: what a router or NAS says about itself, as pages.

use iced::Element;

use crate::app::{LanPing, Message};
use crate::snmp_view::snmp_page;
use crate::syslog_view::syslog_page;
use crate::ui::{page_strip, tabbed_panel};
use crate::upnp_tab::upnp_page;

/// The pages of the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RouterPage {
    #[default]
    Snmp,
    Upnp,
    Syslog,
}

impl RouterPage {
    const ALL: [Self; 3] = [Self::Snmp, Self::Upnp, Self::Syslog];

    fn title(self) -> &'static str {
        match self {
            Self::Snmp => "SNMP",
            Self::Upnp => "UPnP",
            Self::Syslog => "Syslog",
        }
    }
}

pub fn router_panel(app: &LanPing) -> Element<'_, Message> {
    let strip = page_strip(
        &RouterPage::ALL,
        app.router_page,
        RouterPage::title,
        Message::RouterPagePicked,
    );
    let page = match app.router_page {
        RouterPage::Snmp => snmp_page(app),
        RouterPage::Upnp => upnp_page(app),
        RouterPage::Syslog => syslog_page(app),
    };
    tabbed_panel(strip, page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_page_has_its_own_title() {
        let titles: Vec<&str> = RouterPage::ALL.iter().map(|page| page.title()).collect();
        assert_eq!(titles, ["SNMP", "UPnP", "Syslog"]);
    }
}
