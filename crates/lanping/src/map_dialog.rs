//! The network map: the gateway, then every other device grouped by type.
//! A LAN scan cannot see how devices are wired, so no links are drawn.

use std::net::Ipv4Addr;

use iced::widget::{Column, Row, button, column, row, scrollable, space, text};
use iced::{Alignment, Element, Font, Length};
use lanping_core::Host;
use lanping_core::device_type::{DeviceType, guess_device_type};

use crate::app::{LanPing, Message};
use crate::results::device_type_icon;
use crate::settings_page::small_button;
use crate::theme::{self, BOLD, ITEM_SPACING, MUTED};

const TILE_WIDTH: f32 = 170.0;
const SCROLLBAR_GAP: f32 = 16.0;
const NO_NAME: &str = "Unknown device";

pub fn map_dialog(app: &LanPing, gateway: Option<Ipv4Addr>) -> Element<'_, Message> {
    let is_gateway = |host: &&Host| Some(host.ip) == gateway;
    let mut sections = Column::new().spacing(ITEM_SPACING + 6.0);
    if let Some(host) = app.hosts.iter().find(is_gateway) {
        sections = sections.push(group(app, "Gateway", vec![host]));
    }
    for device_type in DeviceType::ALL {
        let members: Vec<&Host> = app
            .hosts
            .iter()
            .filter(|host| Some(host.ip) != gateway && guess_device_type(host) == device_type)
            .collect();
        if !members.is_empty() {
            sections = sections.push(group(app, &device_type.to_string(), members));
        }
    }
    let page = iced::widget::container(sections)
        .padding(iced::Padding::ZERO.right(SCROLLBAR_GAP))
        .width(Length::Fill);

    column![
        text("Network map").size(22).font(BOLD),
        text(
            "Devices grouped by type. A scan cannot see how they are wired, so no links are shown."
        )
        .size(13)
        .color(MUTED),
        scrollable(page).width(Length::Fill).height(Length::Fill),
        row![
            space::horizontal(),
            small_button("Close", Some(Message::CloseModal)),
        ],
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn group<'a>(app: &'a LanPing, title: &str, members: Vec<&'a Host>) -> Element<'a, Message> {
    let tiles = members.into_iter().map(|host| tile(app, host));
    column![
        text(title.to_owned()).size(13).color(MUTED),
        Row::with_children(tiles)
            .spacing(8)
            .wrap()
            .vertical_spacing(8),
    ]
    .spacing(6)
    .into()
}

/// One device. Clicking it opens the device details.
fn tile<'a>(app: &'a LanPing, host: &'a Host) -> Element<'a, Message> {
    let key = host.device_key();
    let name = app
        .labels
        .get(&key)
        .or(host.display_name())
        .unwrap_or(NO_NAME);
    let content = row![
        device_type_icon(guess_device_type(host))
            .size(18)
            .color(MUTED),
        column![
            text(name.to_owned()).size(13),
            text(host.ip.to_string())
                .size(12)
                .font(Font::MONOSPACE)
                .color(MUTED),
        ]
        .spacing(2),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    button(content)
        .on_press(Message::ShowHostDetails(key))
        .padding([8.0, 10.0])
        .width(TILE_WIDTH)
        .style(theme::quiet_button)
        .into()
}
