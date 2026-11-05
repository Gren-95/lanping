//! The device detail dialog: what is known about one host and how it showed up in past scans.

use iced::widget::{Row, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Font, Length};
use lanping_core::Host;
use lanping_core::device_type::guess_device_type;
use lanping_core::history::Sighting;
use lanping_core::os_guess::guess_os;
use lanping_core::port_risk::port_hint;
use lanping_core::ports::format_ports;

use crate::app::{LanPing, Message, NoteField};
use crate::host_filter::RowStatus;
use crate::results::pill;
use crate::settings_page::small_button;
use crate::theme::{self, ACCENT, BAD, BOLD, GOOD, ITEM_SPACING, MUTED, NO_VALUE, WARN};
use crate::timestamp::format_unix_utc;

const LABEL_WIDTH: f32 = 90.0;
const SCROLLBAR_GAP: f32 = 16.0;
const RECENT_SCANS_SHOWN: usize = 8;
const CHANGES_SHOWN: usize = 15;
const SECONDS_PER_DAY: u64 = 86_400;

/// When a device was first and last seen in the saved scans, and how often.
#[derive(Debug, PartialEq, Eq)]
pub struct Activity {
    pub first_seen: u64,
    pub last_seen: u64,
    pub seen_in: usize,
}

/// Summary of `sightings`, or `None` when the device is in no saved scan.
pub fn activity(sightings: &[Sighting]) -> Option<Activity> {
    let first_seen = sightings.iter().map(|s| s.finished_at_unix).min()?;
    let last_seen = sightings.iter().map(|s| s.finished_at_unix).max()?;
    Some(Activity {
        first_seen,
        last_seen,
        seen_in: sightings.len(),
    })
}

pub fn host_details_dialog<'a>(app: &'a LanPing, key: &'a str) -> Element<'a, Message> {
    let Some(host) = app.find_host(key) else {
        return text("This device is no longer in the list.")
            .size(14)
            .into();
    };
    let status = app.row_status(host);
    let label = app.labels.get(key);

    let title = label.map_or_else(|| host.ip.to_string(), str::to_owned);
    let mut heading = row![text(title).size(22).font(BOLD)]
        .spacing(ITEM_SPACING)
        .align_y(Alignment::Center);
    if let Some(badge) = status_pill(status) {
        heading = heading.push(badge);
    }

    let content = column![
        heading,
        facts(host, label.is_some()),
        notes_section(app, key),
        ports_section(host),
        history_section(app),
    ]
    .spacing(ITEM_SPACING + 6.0);
    let buttons = row![
        space::horizontal(),
        small_button("Open with", Some(Message::ShowHostActions(key.to_owned()))),
        small_button("Close", Some(Message::CloseModal)),
    ]
    .spacing(ITEM_SPACING);

    let page = container(content).padding(iced::Padding::ZERO.right(SCROLLBAR_GAP));
    column![scrollable(page).height(Length::Fill), buttons]
        .spacing(ITEM_SPACING + 4.0)
        .into()
}

/// The user's own facts about the device. They are saved with the button, like the device name.
fn notes_section<'a>(app: &'a LanPing, key: &str) -> Element<'a, Message> {
    let draft = &app.note_draft;
    let field =
        |label: &'static str, placeholder: &'static str, value: &'a str, field: NoteField| {
            row![
                text(label).size(13).color(MUTED).width(LABEL_WIDTH),
                text_input(placeholder, value)
                    .on_input(move |typed| Message::NoteDraftChanged(field, typed))
                    .padding(theme::CONTROL_PADDING)
                    .size(13)
                    .style(theme::input),
            ]
            .spacing(ITEM_SPACING)
            .align_y(Alignment::Center)
        };
    column![
        text("Your notes").size(13).color(MUTED),
        field("Owner", "Who it belongs to", &draft.owner, NoteField::Owner),
        field(
            "Location",
            "Where it is",
            &draft.location,
            NoteField::Location
        ),
        field("Tags", "Separated by commas", &draft.tags, NoteField::Tags),
        field("Notes", "Anything else", &draft.notes, NoteField::Notes),
        row![
            space::horizontal(),
            small_button("Save notes", Some(Message::SaveNote(key.to_owned()))),
        ],
    ]
    .spacing(6)
    .into()
}

fn facts(host: &Host, has_label: bool) -> Element<'_, Message> {
    let latency = host.latency.map_or_else(
        || "No reply (ARP only)".to_owned(),
        |latency| format!("{:.1} ms", latency.as_secs_f64() * 1000.0),
    );
    let mut lines = column![].spacing(6);
    if has_label {
        lines = lines.push(fact("Address", host.ip.to_string()));
    }
    lines = lines
        .push(fact(
            "Hostname",
            host.hostname.clone().unwrap_or_else(|| NO_VALUE.to_owned()),
        ))
        .push(fact("IPv6", ipv6_text(host)))
        .push(fact(
            "MAC",
            host.mac.clone().unwrap_or_else(|| NO_VALUE.to_owned()),
        ))
        .push(fact(
            "Vendor",
            host.vendor.clone().unwrap_or_else(|| NO_VALUE.to_owned()),
        ))
        .push(fact("Latency", latency));
    let announced = [
        Some(("Type", format!("{} (best guess)", guess_device_type(host)))),
        guess_os(host).map(|os| ("OS", format!("{os} (best guess)"))),
        host.discovery.name.clone().map(|name| ("Announces", name)),
        device_model(host).map(|model| ("Model", model)),
        host.snmp.serial.clone().map(|serial| ("Serial", serial)),
        host.snmp.location.clone().map(|place| ("Location", place)),
        host.snmp
            .uptime_seconds
            .map(|seconds| ("Uptime", format!("{} days", seconds / SECONDS_PER_DAY))),
        host.snmp
            .interface_count
            .map(|count| ("Interfaces", count.to_string())),
        host.snmp.description.clone().map(|about| ("SNMP", about)),
        (!host.discovery.services.is_empty())
            .then(|| ("Services", host.discovery.services.join(", "))),
    ];
    for (label, value) in announced.into_iter().flatten() {
        lines = lines.push(fact(label, value));
    }
    lines.into()
}

/// Manufacturer and model as the device reports them.
fn device_model(host: &Host) -> Option<String> {
    let model = host.discovery.model.as_ref().or(host.snmp.model.as_ref());
    let parts: Vec<&str> = [host.discovery.manufacturer.as_ref(), model]
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

fn ipv6_text(host: &Host) -> String {
    if host.ipv6.is_empty() {
        return NO_VALUE.to_owned();
    }
    host.ipv6
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("\n")
}

fn fact<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    row![
        text(label).size(13).color(MUTED).width(LABEL_WIDTH),
        text(value).size(14)
    ]
    .spacing(ITEM_SPACING)
    .into()
}

fn ports_section(host: &Host) -> Element<'_, Message> {
    let heading = text("Open ports").size(13).color(MUTED);
    if host.open_ports.is_empty() {
        return column![heading, text("None found").size(14)]
            .spacing(6)
            .into();
    }
    let lines = host
        .open_ports
        .iter()
        .fold(column![].spacing(4), |list, &port| {
            let banner = host.banner(port).unwrap_or_default();
            list.push(
                Row::with_children(vec![
                    pill(
                        port.to_string(),
                        if port_hint(port).is_some() {
                            WARN
                        } else {
                            ACCENT
                        },
                    ),
                    text(banner.to_owned())
                        .font(Font::MONOSPACE)
                        .size(12)
                        .color(MUTED)
                        .into(),
                    text(port_hint(port).unwrap_or_default())
                        .size(12)
                        .color(WARN)
                        .into(),
                ])
                .spacing(ITEM_SPACING)
                .align_y(Alignment::Center),
            )
        });
    let mut section = column![heading, lines].spacing(6);
    if host
        .open_ports
        .iter()
        .any(|&port| port_hint(port).is_some())
    {
        section = section.push(
            text("Amber ports are hints to check, based only on the port number. This is not a security audit.")
                .size(12)
                .color(MUTED),
        );
    }
    section.into()
}

fn history_section(app: &LanPing) -> Element<'_, Message> {
    let heading = text("History").size(13).color(MUTED);
    let Some(summary) = activity(&app.detail_sightings) else {
        return column![heading, text("Not in any saved scan yet").size(14)]
            .spacing(6)
            .into();
    };
    let summary_lines = column![
        fact("First seen", format_unix_utc(summary.first_seen)),
        fact("Last seen", format_unix_utc(summary.last_seen)),
        fact(
            "Seen in",
            format!(
                "{} of {} saved scans",
                summary.seen_in, app.detail_scan_count
            )
        ),
    ]
    .spacing(6);
    let recent = app.detail_sightings.iter().take(RECENT_SCANS_SHOWN).fold(
        column![].spacing(4),
        |list, sighting| {
            let ports = if sighting.host.open_ports.is_empty() {
                NO_VALUE.to_owned()
            } else {
                format_ports(&sighting.host.open_ports)
            };
            let latency = sighting.host.latency.map_or_else(
                || NO_VALUE.to_owned(),
                |l| format!("{:.1} ms", l.as_secs_f64() * 1000.0),
            );
            list.push(
                text(format!(
                    "{}   {}   {}   ports: {ports}",
                    format_unix_utc(sighting.finished_at_unix),
                    sighting.target,
                    latency
                ))
                .size(12)
                .font(Font::MONOSPACE),
            )
        },
    );
    let changes = app
        .detail_timeline
        .iter()
        .take(CHANGES_SHOWN)
        .fold(column![].spacing(4), |list, entry| {
            list.push(fact_at(entry.at_unix, entry.change.to_string()))
        });
    column![
        heading,
        summary_lines,
        text("Changes").size(13).color(MUTED),
        changes,
        text("Recent scans").size(13).color(MUTED),
        recent
    ]
    .spacing(6)
    .into()
}

fn fact_at<'a>(at_unix: u64, what: String) -> Element<'a, Message> {
    row![
        text(format_unix_utc(at_unix))
            .size(12)
            .font(Font::MONOSPACE)
            .color(MUTED),
        text(what).size(13),
    ]
    .spacing(ITEM_SPACING)
    .align_y(Alignment::Center)
    .into()
}

fn status_pill<'a>(status: RowStatus) -> Option<Element<'a, Message>> {
    let (label, tint) = match status {
        RowStatus::Unchanged => return None,
        RowStatus::New => ("new", GOOD),
        RowStatus::Changed => ("changed", WARN),
        RowStatus::Gone => ("gone", BAD),
    };
    Some(pill(label, tint))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sighting(finished_at_unix: u64) -> Sighting {
        Sighting {
            finished_at_unix,
            target: "10.0.0.0/24".into(),
            host: Host::new("10.0.0.5".parse().unwrap()),
        }
    }

    #[test]
    fn summarizes_first_and_last_sighting() {
        assert_eq!(activity(&[]), None);
        let summary = activity(&[sighting(300), sighting(100), sighting(200)]).unwrap();
        assert_eq!(
            summary,
            Activity {
                first_seen: 100,
                last_seen: 300,
                seen_in: 3
            }
        );
    }
}
