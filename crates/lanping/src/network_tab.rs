//! The My network tab: this computer's addresses, gateway, DNS servers and Wi-Fi name, and
//! on request the public address, IPv6 and sign-in page checks.

use iced::widget::{button, column, row, text};
use iced::{Alignment, Element, Length, Task};
use lanping_core::my_network::{InterfaceInfo, LocalFacts, local_facts};
use lanping_core::online_checks::{CONTACTS, Ipv6Status, OnlineFacts, PortalStatus, check_online};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, lucide, with_tooltip};
use crate::job::Job;
use crate::results::pill;
use crate::theme::{self, BAD, BOLD, GOOD, MUTED, SECTION_SPACING, WARN};
use crate::ui::{
    FACT_LABEL_WIDTH, card, empty_state, fact_row, fact_row_copying, note, page_header, panel,
    scrolling,
};

const SECTION_TITLE_SIZE: u32 = 15;
const NO_VALUE_TEXT: &str = "Not available";

enum Online {
    Idle,
    Running,
    Done(OnlineFacts),
}

/// What was read from this computer, and what the internet checks found.
pub struct NetworkState {
    local: Option<LocalFacts>,
    online: Online,
    local_job: Job,
    online_job: Job,
}

impl NetworkState {
    pub fn new() -> Self {
        Self {
            local: None,
            online: Online::Idle,
            local_job: Job::default(),
            online_job: Job::default(),
        }
    }

    pub fn has_read_local(&self) -> bool {
        self.local.is_some()
    }

    pub fn read_local(&mut self) -> Task<Message> {
        self.local_job
            .spawn(async { local_facts().await }, Message::NetworkLocalDone)
    }

    pub fn finish_local(&mut self, generation: u64, facts: LocalFacts) {
        if self.local_job.is_current(generation) {
            self.local_job.finished();
            self.local = Some(facts);
        }
    }

    pub fn check_online(&mut self) -> Task<Message> {
        self.online = Online::Running;
        self.online_job
            .spawn(async { check_online().await }, Message::NetworkOnlineDone)
    }

    pub fn finish_online(&mut self, generation: u64, facts: OnlineFacts) {
        if self.online_job.is_current(generation) {
            self.online_job.finished();
            self.online = Online::Done(facts);
        }
    }
}

pub fn network_panel(app: &LanPing) -> Element<'_, Message> {
    let network = &app.network;
    let refresh = with_tooltip(
        button(
            row![
                lucide::refresh_cw().size(ICON_SIZE),
                text("Refresh").size(14)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(Message::NetworkRefresh)
        .padding([7.0, 20.0])
        .style(theme::quiet_button),
        "Read the addresses and settings of this computer again",
    );
    let header = page_header(
        "My network",
        "This computer's addresses, gateway, DNS servers and Wi-Fi.",
        Some(refresh),
    );
    let content: Element<'_, Message> = match &network.local {
        None => empty_state("Reading", "Looking at this computer's network settings."),
        Some(local) => scrolling(
            column![local_card(local), internet_card(&network.online)].spacing(SECTION_SPACING),
        )
        .into(),
    };
    panel(
        column![header, content]
            .spacing(SECTION_SPACING)
            .height(Length::Fill),
    )
}

fn local_card(local: &LocalFacts) -> Element<'_, Message> {
    let value = |text: Option<String>| text.unwrap_or_else(|| NO_VALUE_TEXT.to_owned());
    let dns = (!local.dns_servers.is_empty()).then(|| {
        local
            .dns_servers
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    });
    let mut facts = column![
        text("This computer").size(SECTION_TITLE_SIZE).font(BOLD),
        fact_row("Wi-Fi", value(local.wifi_name.clone())),
        fact_row(
            "Gateway",
            value(local.gateway.map(|gateway| gateway.to_string()))
        ),
        fact_row("DNS servers", value(dns)),
    ]
    .spacing(6);
    for interface in &local.interfaces {
        facts = facts.push(interface_block(interface));
    }
    card(facts)
}

fn interface_block(interface: &InterfaceInfo) -> Element<'_, Message> {
    interface
        .addresses
        .iter()
        .fold(
            column![text(interface.name.as_str()).size(13).color(MUTED)].spacing(6),
            |block, address| {
                let note = if address.is_link_local() {
                    "  (only on this link)"
                } else {
                    ""
                };
                block.push(fact_row_copying(
                    if address.ip.is_ipv4() { "IPv4" } else { "IPv6" },
                    format!("{}{note}", address.with_prefix()),
                    address.ip.to_string(),
                ))
            },
        )
        .padding([8.0, 0.0])
        .into()
}

fn internet_card(online: &Online) -> Element<'_, Message> {
    let busy = matches!(online, Online::Running);
    let check = button(text("Check the internet").size(14))
        .on_press_maybe((!busy).then_some(Message::NetworkCheckOnline))
        .padding([7.0, 20.0])
        .style(theme::primary_button);
    let mut body = column![
        text("The internet").size(SECTION_TITLE_SIZE).font(BOLD),
        note(format!(
            "Pressing the button contacts {CONTACTS}. Nothing is sent unless you press it."
        )),
        check,
    ]
    .spacing(8);
    match online {
        Online::Idle => {}
        Online::Running => body = body.push(note("Checking.")),
        Online::Done(facts) => body = body.push(online_rows(facts)),
    }
    card(body)
}

fn online_rows(facts: &OnlineFacts) -> Element<'_, Message> {
    let public = match &facts.public_address {
        Ok(address) => fact_row("Public address", address.to_string()),
        Err(error) => status_row("Public address", pill("Unknown", WARN), error.clone()),
    };
    let ipv6 = match &facts.ipv6 {
        Ipv6Status::Works => status_row("IPv6", pill("Works", GOOD), String::new()),
        Ipv6Status::Fails(reason) => status_row(
            "IPv6",
            pill("Not available", WARN),
            format!("{reason}. This network may not offer IPv6."),
        ),
    };
    let portal = match &facts.captive_portal {
        PortalStatus::Open => {
            status_row("Sign-in page", pill("None in the way", GOOD), String::new())
        }
        PortalStatus::SignInNeeded { location } => status_row(
            "Sign-in page",
            pill("Sign-in needed", BAD),
            location
                .clone()
                .map(|location| format!("Open {location} in a browser to sign in."))
                .unwrap_or_else(|| "A page answered in place of the internet.".to_owned()),
        ),
        PortalStatus::Unknown(reason) => {
            status_row("Sign-in page", pill("Unknown", WARN), reason.clone())
        }
    };
    column![public, ipv6, portal].spacing(6).into()
}

/// A label, a coloured verdict and a line of explanation under it.
fn status_row<'a>(
    label: &'static str,
    verdict: Element<'a, Message>,
    explanation: String,
) -> Element<'a, Message> {
    let mut body = column![verdict].spacing(4).width(Length::Fill);
    if !explanation.is_empty() {
        body = body.push(text(explanation).size(12).color(MUTED));
    }
    row![text(label).color(MUTED).width(FACT_LABEL_WIDTH), body]
        .spacing(8)
        .align_y(Alignment::Start)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_overtaken_local_read_is_ignored() {
        let mut state = NetworkState::new();
        let _ = state.read_local();
        let _ = state.read_local();
        let facts = LocalFacts {
            interfaces: Vec::new(),
            gateway: None,
            dns_servers: Vec::new(),
            wifi_name: None,
        };
        state.finish_local(1, facts.clone());
        assert!(!state.has_read_local());
        state.finish_local(2, facts);
        assert!(state.has_read_local());
    }

    #[test]
    fn an_overtaken_online_check_is_ignored() {
        let mut state = NetworkState::new();
        let _ = state.check_online();
        let _ = state.check_online();
        let facts = OnlineFacts {
            public_address: Err("x".to_owned()),
            ipv6: Ipv6Status::Works,
            captive_portal: PortalStatus::Open,
        };
        state.finish_online(1, facts.clone());
        assert!(matches!(state.online, Online::Running));
        state.finish_online(2, facts);
        assert!(matches!(state.online, Online::Done(_)));
    }
}
