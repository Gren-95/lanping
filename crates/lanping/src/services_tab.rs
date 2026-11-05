//! The Services tab: what announces itself on the network, grouped by kind.

use std::collections::HashSet;
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
use crate::ui::{empty_state, page_header, scrolling};

const CARD_PADDING: f32 = 12.0;
const GROUP_SPACING: f32 = 8.0;
const DETAIL_KEY_WIDTH: f32 = 150.0;

enum Lookup {
    Idle,
    Running,
    Done(Vec<ServiceEntry>),
}

/// What the last browse found.
pub struct ServicesState {
    lookup: Lookup,
    /// Entries whose text records are open, by [`entry_key`].
    expanded: HashSet<String>,
    job: Job,
}

impl ServicesState {
    pub fn new() -> Self {
        Self {
            lookup: Lookup::Idle,
            expanded: HashSet::new(),
            job: Job::default(),
        }
    }

    /// Opens the text records of an entry, or closes them.
    pub fn toggle(&mut self, key: String) {
        if !self.expanded.remove(&key) {
            self.expanded.insert(key);
        }
    }

    pub fn start(&mut self) -> Task<Message> {
        self.lookup = Lookup::Running;
        self.expanded.clear();
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

pub fn services_page(app: &LanPing) -> Element<'_, Message> {
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
        Lookup::Done(entries) => {
            scrolling(results(entries, &app.hosts, &app.services.expanded)).into()
        }
    };
    column![header, content]
        .spacing(SECTION_SPACING)
        .height(Length::Fill)
        .into()
}

fn results<'a>(
    entries: &'a [ServiceEntry],
    hosts: &[Host],
    expanded: &HashSet<String>,
) -> Element<'a, Message> {
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
                    list.push(entry_card(entry, hosts, expanded))
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

fn entry_card<'a>(
    entry: &'a ServiceEntry,
    hosts: &[Host],
    expanded: &HashSet<String>,
) -> Element<'a, Message> {
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
    let key = entry_key(entry);
    let open = expanded.contains(&key);
    let mut header = row![container(title.wrap().vertical_spacing(4)).width(Length::Fill)]
        .spacing(8)
        .align_y(Alignment::Center);
    if !entry.details.is_empty() {
        header = header.push(details_toggle(entry.details.len(), open, key));
    }
    let mut body = column![header].spacing(8);
    if open {
        body = body.push(details_list(&entry.details));
    }
    container(body)
        .padding(CARD_PADDING)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

/// What tells one entry from another, so its open or closed state survives a redraw.
fn entry_key(entry: &ServiceEntry) -> String {
    format!(
        "{}|{}|{}",
        entry.kind,
        entry.name,
        entry
            .address
            .map(|address| address.to_string())
            .unwrap_or_default()
    )
}

fn details_toggle<'a>(count: usize, open: bool, key: String) -> Element<'a, Message> {
    let chevron = if open {
        lucide::chevron_down()
    } else {
        lucide::chevron_right()
    };
    button(
        row![
            chevron.size(ICON_SIZE - 3.0).color(MUTED),
            text(format!("Details ({count})")).size(13)
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .on_press(Message::ServicesToggle(key))
    .padding([5.0, 12.0])
    .style(theme::quiet_button)
    .into()
}

/// Every text record on its own line, as `key` and `value`.
fn details_list(details: &[String]) -> Element<'_, Message> {
    details
        .iter()
        .fold(column![].spacing(4), |list, detail| {
            let (key, value) = detail.split_once('=').unwrap_or(("", detail));
            list.push(
                row![
                    text(key).size(12).color(MUTED).width(DETAIL_KEY_WIDTH),
                    text(value).size(12).width(Length::Fill),
                ]
                .spacing(8),
            )
        })
        .into()
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
    fn an_entry_opens_and_closes_by_its_key() {
        let mut state = ServicesState::new();
        let key = entry_key(&entry("_x._tcp", "a", [10, 0, 0, 1]));
        state.toggle(key.clone());
        assert!(state.expanded.contains(&key));
        state.toggle(key.clone());
        assert!(!state.expanded.contains(&key));
        assert_ne!(key, entry_key(&entry("_x._tcp", "a", [10, 0, 0, 2])));
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
