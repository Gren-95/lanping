//! The Subnet tab: what an address with a prefix length or netmask covers.

use iced::font::Weight;
use iced::widget::{button, column, container, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Font, Length};
use lanping_core::subnet::{Subnet, SubnetError, describe};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, copy_button, lucide, with_tooltip};
use crate::theme::{self, BAD, Edge, MUTED};

const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};
const PANEL_PADDING: f32 = 16.0;
const SECTION_SPACING: f32 = 14.0;
const LABEL_WIDTH: f32 = 150.0;
const ROW_PADDING: [f32; 2] = [8.0, 12.0];

/// What is typed in the input field.
#[derive(Debug, Default)]
pub struct SubnetState {
    input: String,
}

impl SubnetState {
    pub fn set_input(&mut self, input: String) {
        self.input = input;
    }
}

pub fn subnet_panel(app: &LanPing) -> Element<'_, Message> {
    let input = text_input(
        "192.168.1.10/24 or 10.0.0.1 255.255.255.0",
        &app.subnet.input,
    )
    .on_input(Message::SubnetInputChanged)
    .padding(theme::CONTROL_PADDING)
    .size(13)
    .width(Length::Fill)
    .style(theme::input_at(Edge::Alone));
    let result: Element<'_, Message> = match describe(&app.subnet.input) {
        Ok(subnet) => details(subnet),
        Err(SubnetError::Empty) => hint(SubnetError::Empty.to_string(), MUTED),
        Err(error) => hint(error.to_string(), BAD),
    };
    let page = column![input, result]
        .spacing(SECTION_SPACING)
        .height(Length::Fill);
    container(container(page).padding(PANEL_PADDING).height(Length::Fill))
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn hint(message: String, color: iced::Color) -> Element<'static, Message> {
    container(text(message).size(14).color(color))
        .center_x(Length::Fill)
        .padding(24)
        .into()
}

fn details<'a>(subnet: Subnet) -> Element<'a, Message> {
    let mut header = row![
        text(subnet.network.clone()).size(22).font(BOLD),
        copy_button(subnet.network.clone(), "network"),
        space::horizontal(),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if let Some(target) = subnet.scan_target {
        header = header.push(with_tooltip(
            button(lucide::scan_search().size(ICON_SIZE + 2.0))
                .on_press(Message::SubnetScanRange(target))
                .padding([7.0, 20.0])
                .style(theme::primary_button),
            "Scan this range",
        ));
    }
    let lines = subnet
        .rows
        .into_iter()
        .enumerate()
        .fold(column![], |list, (index, line)| {
            let value = line.value;
            let cells = row![
                text(line.label).color(MUTED).width(LABEL_WIDTH),
                text(value.clone()).font(BOLD),
                copy_button(value, line.label),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            list.push(
                container(cells)
                    .padding(ROW_PADDING)
                    .width(Length::Fill)
                    .style(theme::stripe(index % 2 == 1)),
            )
        });
    column![
        header,
        scrollable(lines).width(Length::Fill).height(Length::Fill)
    ]
    .spacing(SECTION_SPACING)
    .into()
}
