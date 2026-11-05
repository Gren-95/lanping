//! The UPnP tab: the port forwards the router has open, read-only.

use iced::widget::{Row as LayoutRow, button, column, container, row, text};
use iced::{Alignment, Element, Length, Task};
use lanping_core::Host;
use lanping_core::upnp::{PortMapping, UpnpError, UpnpReport, read_port_mappings};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, lucide, with_tooltip};
use crate::job::Job;
use crate::results::pill;
use crate::snmp_tab::format_uptime;
use crate::theme::{self, ACCENT, BOLD, GOOD, MUTED, SECTION_SPACING, WARN};
use crate::ui::{empty_state, note, page_header, panel, scrolling};

const CARD_PADDING: f32 = 12.0;

enum Lookup {
    Idle,
    Running,
    Done(UpnpReport),
    Failed(UpnpError),
}

/// What the router said the last time it was asked.
pub struct UpnpState {
    lookup: Lookup,
    job: Job,
}

impl UpnpState {
    pub fn new() -> Self {
        Self {
            lookup: Lookup::Idle,
            job: Job::default(),
        }
    }

    pub fn start(&mut self) -> Task<Message> {
        self.lookup = Lookup::Running;
        self.job
            .spawn(async { read_port_mappings().await }, Message::UpnpDone)
    }

    pub fn finish(&mut self, generation: u64, result: Result<UpnpReport, UpnpError>) {
        if !self.job.is_current(generation) {
            return;
        }
        self.job.finished();
        self.lookup = match result {
            Ok(report) => Lookup::Done(report),
            Err(error) => Lookup::Failed(error),
        };
    }
}

pub fn upnp_panel(app: &LanPing) -> Element<'_, Message> {
    let busy = matches!(app.upnp.lookup, Lookup::Running);
    let read = with_tooltip(
        button(
            row![
                lucide::network().size(ICON_SIZE),
                text("Read port forwards").size(14)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press_maybe((!busy).then_some(Message::UpnpRead))
        .padding([7.0, 20.0])
        .style(theme::primary_button),
        "Ask the router which ports it forwards. Nothing is changed.",
    );
    let header = page_header(
        "Port forwards",
        "The ports your router has open to the internet, as programs asked for them over UPnP.",
        Some(read),
    );
    let content: Element<'_, Message> = match &app.upnp.lookup {
        Lookup::Idle => empty_state(
            "Nothing read yet",
            "Press Read port forwards. This only looks; nothing is added or removed.",
        ),
        Lookup::Running => empty_state("Asking the router", "This takes a few seconds."),
        Lookup::Failed(error) => empty_state("Cannot read port forwards", error.to_string()),
        Lookup::Done(report) => scrolling(report_view(report, &app.hosts)).into(),
    };
    panel(
        column![header, content]
            .spacing(SECTION_SPACING)
            .height(Length::Fill),
    )
}

fn report_view<'a>(report: &'a UpnpReport, hosts: &[Host]) -> Element<'a, Message> {
    let router = report
        .device
        .friendly_name
        .clone()
        .unwrap_or_else(|| "Router".to_owned());
    let count = report.mappings.len();
    let summary = if count == 0 {
        pill("No ports forwarded", GOOD)
    } else {
        pill(format!("{count} forwarded"), ACCENT)
    };
    let mut page = column![
        row![
            summary,
            text(format!("{router}  ·  {}", report.gateway))
                .size(13)
                .color(MUTED)
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(SECTION_SPACING);
    if count == 0 {
        page = page.push(note(
            "The router has no forwards open. Programs that need one, such as game consoles, may \
             have none, or UPnP may only be answering and not offering them.",
        ));
    }
    for mapping in &report.mappings {
        page = page.push(mapping_card(mapping, hosts));
    }
    if report.truncated {
        page = page.push(note("The router has more forwards than are shown here."));
    }
    page.into()
}

fn mapping_card<'a>(mapping: &'a PortMapping, hosts: &[Host]) -> Element<'a, Message> {
    let title = format!(
        "{} {}  →  {}:{}",
        mapping.protocol, mapping.external_port, mapping.internal_client, mapping.internal_port
    );
    let mut badges: Vec<Element<'a, Message>> = Vec::new();
    if !mapping.enabled {
        badges.push(pill("Disabled", WARN));
    }
    if is_gone(mapping, hosts) {
        badges.push(pill("Device not on the network", WARN));
    }
    let detail = [
        (!mapping.description.is_empty()).then(|| mapping.description.clone()),
        Some(lease_text(mapping.lease_seconds)),
        (!mapping.remote_host.is_empty()).then(|| format!("only from {}", mapping.remote_host)),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("  ·  ");
    let heading =
        LayoutRow::with_children(std::iter::once(text(title).font(BOLD).into()).chain(badges))
            .spacing(8)
            .align_y(Alignment::Center)
            .wrap()
            .vertical_spacing(6);
    container(column![heading, text(detail).size(12).color(MUTED)].spacing(4))
        .padding(CARD_PADDING)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

fn lease_text(lease_seconds: u32) -> String {
    match lease_seconds {
        0 => "does not expire".to_owned(),
        seconds => format!("expires in {}", format_uptime(u64::from(seconds))),
    }
}

/// A forward to an address that no scanned device has. Without a scan there is nothing to
/// compare with, so nothing is flagged.
fn is_gone(mapping: &PortMapping, hosts: &[Host]) -> bool {
    match mapping.internal_ip() {
        Some(ip) if !hosts.is_empty() => hosts.iter().all(|host| host.ip != ip),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn mapping(client: &str) -> PortMapping {
        PortMapping {
            protocol: "TCP".to_owned(),
            external_port: 22,
            internal_client: client.to_owned(),
            internal_port: 22,
            description: String::new(),
            enabled: true,
            lease_seconds: 0,
            remote_host: String::new(),
        }
    }

    #[test]
    fn a_forward_to_an_unlisted_device_is_flagged_only_after_a_scan() {
        let known = Host::new(Ipv4Addr::new(192, 168, 1, 5));
        assert!(!is_gone(&mapping("192.168.1.9"), &[]), "no scan, no flag");
        assert!(is_gone(
            &mapping("192.168.1.9"),
            std::slice::from_ref(&known)
        ));
        assert!(!is_gone(
            &mapping("192.168.1.5"),
            std::slice::from_ref(&known)
        ));
        assert!(
            !is_gone(&mapping("nas"), std::slice::from_ref(&known)),
            "no address to compare"
        );
    }

    #[test]
    fn leases_say_when_they_end() {
        assert_eq!(lease_text(0), "does not expire");
        assert_eq!(lease_text(3600), "expires in 1h 0m");
    }

    #[test]
    fn an_overtaken_answer_is_ignored() {
        let mut state = UpnpState::new();
        let _ = state.start();
        let _ = state.start();
        state.finish(1, Err(UpnpError::NoAnswer));
        assert!(matches!(state.lookup, Lookup::Running));
        state.finish(2, Err(UpnpError::NoAnswer));
        assert!(matches!(state.lookup, Lookup::Failed(UpnpError::NoAnswer)));
    }
}
