//! The summary dialog: totals across the listed devices.

use iced::font::Weight;
use iced::widget::{Column, Row, column, row, scrollable, space, text};
use iced::{Alignment, Element, Font, Length};
use lanping_core::summary::Summary;

use crate::app::Message;
use crate::results::device_type_icon;
use crate::settings_page::small_button;
use crate::theme::MUTED;

const ITEM_SPACING: f32 = 10.0;
const SECTION_WIDTH: f32 = 170.0;
const ROWS_SHOWN: usize = 8;
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};

pub fn summary_dialog(summary: &Summary) -> Element<'_, Message> {
    let by_type = summary
        .by_type
        .iter()
        .take(ROWS_SHOWN)
        .map(|(kind, count)| {
            count_row(
                row![
                    device_type_icon(*kind).size(14).color(MUTED),
                    text(kind.to_string()).size(13),
                ]
                .spacing(6)
                .align_y(Alignment::Center)
                .into(),
                *count,
            )
        });
    let by_vendor = summary
        .by_vendor
        .iter()
        .take(ROWS_SHOWN)
        .map(|(vendor, count)| count_row(text(vendor.clone()).size(13).into(), *count));
    let by_port = summary
        .by_port
        .iter()
        .take(ROWS_SHOWN)
        .map(|(port, count)| count_row(text(format!("Port {port}")).size(13).into(), *count));

    let sections = Row::with_children(vec![
        section("By type", by_type.collect()),
        section("By vendor", by_vendor.collect()),
        section("Open ports", by_port.collect()),
    ])
    .spacing(ITEM_SPACING + 10.0)
    .wrap()
    .vertical_spacing(ITEM_SPACING + 6.0);

    column![
        text("Summary").size(22).font(BOLD),
        text(format!(
            "{} devices listed, {} first seen in the last 7 days",
            summary.device_count, summary.new_this_week
        ))
        .size(14),
        scrollable(sections).height(Length::Shrink),
        row![
            space::horizontal(),
            small_button("Close", Some(Message::CloseModal)),
        ],
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn section<'a>(title: &'static str, rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let body: Element<'a, Message> = if rows.is_empty() {
        text("None").size(13).color(MUTED).into()
    } else {
        Column::with_children(rows).spacing(4).into()
    };
    column![text(title).size(13).color(MUTED), body]
        .spacing(6)
        .width(SECTION_WIDTH)
        .into()
}

fn count_row(label: Element<'_, Message>, count: usize) -> Element<'_, Message> {
    row![
        label,
        space::horizontal(),
        text(count.to_string()).size(13).font(BOLD)
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}
