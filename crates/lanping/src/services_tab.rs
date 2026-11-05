//! The Services tab: what announces itself on the network, grouped by kind.

use std::net::Ipv4Addr;

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length, Task};
use lanping_core::Host;
use lanping_core::service_browse::{ServiceEntry, browse, kind_title};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, copy_button, lucide, with_tooltip};
use crate::job::Job;
use crate::settings_page::small_button;
use crate::theme::{self, BOLD, MUTED, SECTION_SPACING};
use crate::ui::{empty_state, page_header, panel, scrolling};

const CARD_PADDING: f32 = 12.0;
const GROUP_SPACING: f32 = 8.0;

enum Lookup {
    Idle,
    Running,
    Done(Vec<ServiceEntry>),
}

/// What the last browse found.
pub struct ServicesState {
    lookup: Lookup,
    job: Job,
}

impl ServicesState {
    pub fn new() -> Self {
        Self {
            lookup: Lookup::Idle,
            job: Job::default(),
        }
    }

    pub fn start(&mut self) -> Task<Message> {
        self.lookup = Lookup::Running;
        self.job
            .spawn(async { browse().await }, Message::ServicesDone)
    }

    pub fn finish(&mut self, generation: u64, entries: Vec<ServiceEntry>) {
        if !self.job.is_current(generation) {
            return;
        }
        self.job.finished();
        self.lookup = Lookup::Done(entries);
    }
}

pub fn services_panel(app: &LanPing) -> Element<'_, Message> {
    let busy = matches!(app.services.lookup, Lookup::Running);
    let browse_button = with_tooltip(
        button(
            row![
                lucide::radio().size(ICON_SIZE),
                text("Browse the network").size(14)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press_maybe((!busy).then_some(Message::ServicesBrowse))
        .padding([7.0, 20.0])
        .style(theme::primary_button),
        "Ask the network what announces itself. It listens for a few seconds.",
    );
    let header = page_header(
        "Services",
        "Printers, Chromecasts, AirPlay, file shares and media servers that announce themselves.",
        Some(browse_button),
    );
    let content: Element<'_, Message> = match &app.services.lookup {
        Lookup::Idle => empty_state(
            "Nothing browsed yet",
            "Press Browse the network. It only listens to what devices say about themselves.",
        ),
        Lookup::Running => empty_state("Listening", "This takes a few seconds."),
        Lookup::Done(entries) if entries.is_empty() => empty_state(
            "Nothing announced itself",
            "Some networks block multicast between devices, especially guest Wi-Fi.",
        ),
        Lookup::Done(entries) => scrolling(results(entries, &app.hosts)).into(),
    };
    panel(
        column![header, content]
            .spacing(SECTION_SPACING)
            .height(Length::Fill),
    )
}

fn results<'a>(entries: &'a [ServiceEntry], hosts: &[Host]) -> Element<'a, Message> {
    groups(entries)
        .into_iter()
        .fold(column![].spacing(SECTION_SPACING), |page, group| {
            let heading = row![
                text(group.title).font(BOLD),
                text(group.entries.len().to_string()).size(13).color(MUTED),
                text(group.kind).size(12).color(MUTED),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            let cards = group
                .entries
                .into_iter()
                .fold(column![].spacing(GROUP_SPACING), |list, entry| {
                    list.push(entry_card(entry, hosts))
                });
            page.push(column![heading, cards].spacing(GROUP_SPACING))
        })
        .into()
}

/// The entries of one kind, under the plain name of that kind.
struct Group<'a> {
    kind: &'a str,
    title: String,
    entries: Vec<&'a ServiceEntry>,
}

/// The entries split by kind, with the groups in the order of their plain names. Entries
/// arrive sorted, so a kind is one run.
fn groups(entries: &[ServiceEntry]) -> Vec<Group<'_>> {
    let mut groups: Vec<Group<'_>> = Vec::new();
    for entry in entries {
        match groups.last_mut() {
            Some(group) if group.kind == entry.kind => group.entries.push(entry),
            _ => groups.push(Group {
                kind: &entry.kind,
                title: kind_title(&entry.kind),
                entries: vec![entry],
            }),
        }
    }
    groups.sort_by_cached_key(|group| group.title.to_lowercase());
    groups
}

fn entry_card<'a>(entry: &'a ServiceEntry, hosts: &[Host]) -> Element<'a, Message> {
    let place = match (entry.address, entry.port) {
        (Some(address), Some(port)) => format!("{address}:{port}"),
        (Some(address), None) => address.to_string(),
        (None, Some(port)) => format!("port {port}"),
        (None, None) => String::new(),
    };
    let mut title = row![
        text(entry.display_name()).font(BOLD),
        text(place.clone()).size(13).color(MUTED)
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    if !place.is_empty() {
        title = title.push(copy_button(place, "address"));
    }
    if let Some(host) = entry.address.and_then(|address| host_at(hosts, address)) {
        title = title.push(small_button(
            "Show device",
            Some(Message::ShowHostDetails(host.device_key())),
        ));
    }
    let mut body = column![title.wrap().vertical_spacing(4)].spacing(4);
    if let Some(details) = details_line(entry) {
        body = body.push(text(details).size(12).color(MUTED).width(Length::Fill));
    }
    container(body)
        .padding(CARD_PADDING)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

/// The readable text records on one line that wraps, and how many were left out.
fn details_line(entry: &ServiceEntry) -> Option<String> {
    let (shown, hidden) = entry.readable_details();
    if shown.is_empty() && hidden == 0 {
        return None;
    }
    let mut line = shown.join("  ·  ");
    if hidden > 0 {
        if !line.is_empty() {
            line.push_str("  ·  ");
        }
        line.push_str(&format!("{hidden} more not shown"));
    }
    Some(line)
}

fn host_at(hosts: &[Host], address: Ipv4Addr) -> Option<&Host> {
    hosts.iter().find(|host| host.ip == address)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, name: &str, address: [u8; 4]) -> ServiceEntry {
        ServiceEntry {
            kind: kind.to_owned(),
            name: name.to_owned(),
            address: Some(Ipv4Addr::from(address)),
            port: None,
            host: None,
            details: Vec::new(),
        }
    }

    #[test]
    fn entries_are_split_into_one_group_per_kind() {
        let entries = [
            entry("_http._tcp", "a", [10, 0, 0, 1]),
            entry("_http._tcp", "b", [10, 0, 0, 2]),
            entry("_ipp._tcp", "c", [10, 0, 0, 3]),
        ];
        let grouped = groups(&entries);
        let shape: Vec<(&str, usize)> = grouped
            .iter()
            .map(|group| (group.kind, group.entries.len()))
            .collect();
        assert_eq!(
            shape,
            [("_ipp._tcp", 1), ("_http._tcp", 2)],
            "Printers, then Web pages"
        );
        assert!(groups(&[]).is_empty());
    }

    #[test]
    fn the_details_line_lists_what_is_readable_and_counts_the_rest() {
        let mut found = entry("_x._tcp", "a", [10, 0, 0, 1]);
        assert_eq!(details_line(&found), None);
        found.details = vec!["model=Mac16,5".to_owned(), format!("pk={}", "a".repeat(64))];
        assert_eq!(
            details_line(&found).as_deref(),
            Some("model=Mac16,5  ·  1 more not shown")
        );
    }

    #[test]
    fn an_entry_is_linked_to_the_device_with_its_address() {
        let hosts = [Host::new(Ipv4Addr::new(10, 0, 0, 2))];
        assert!(host_at(&hosts, Ipv4Addr::new(10, 0, 0, 2)).is_some());
        assert!(host_at(&hosts, Ipv4Addr::new(10, 0, 0, 9)).is_none());
    }

    #[test]
    fn an_overtaken_answer_is_ignored() {
        let mut state = ServicesState::new();
        let _ = state.start();
        let _ = state.start();
        state.finish(1, Vec::new());
        assert!(matches!(state.lookup, Lookup::Running));
        state.finish(2, Vec::new());
        assert!(matches!(state.lookup, Lookup::Done(_)));
    }
}
