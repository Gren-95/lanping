//! Small widgets that several pages build the same way.

use iced::widget::{
    Row as LayoutRow, Scrollable, button, column, container, row, scrollable, space, text,
};
use iced::{Alignment, Element, Length, Padding};

use crate::app::Message;
use crate::icons::copy_button;
use crate::theme::{self, BOLD, MUTED, PANEL_PADDING, SCROLLBAR_GAP};

const EMPTY_STATE_TITLE_SIZE: u32 = 20;
const EMPTY_STATE_DETAIL_SIZE: u32 = 14;
const EMPTY_STATE_PADDING: f32 = 24.0;

/// A line of muted explanation.
pub fn note<'a>(message: impl Into<String>) -> Element<'a, Message> {
    text(message.into())
        .size(EMPTY_STATE_DETAIL_SIZE)
        .color(MUTED)
        .into()
}

/// A centered title with a muted line under it, for a page that has nothing to show yet.
pub fn empty_state<'a>(title: &'static str, detail: impl Into<String>) -> Element<'a, Message> {
    container(
        column![
            text(title).size(EMPTY_STATE_TITLE_SIZE).font(BOLD),
            text(detail.into())
                .size(EMPTY_STATE_DETAIL_SIZE)
                .color(MUTED),
        ]
        .spacing(6)
        .align_x(Alignment::Center),
    )
    .center(Length::Fill)
    .padding(EMPTY_STATE_PADDING)
    .into()
}

/// The panel every tab's page sits in.
pub fn panel<'a>(page: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(container(page).padding(PANEL_PADDING).height(Length::Fill))
        .style(theme::panel)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Content that scrolls, with room kept for the scrollbar.
pub fn scrolling<'a>(body: impl Into<Element<'a, Message>>) -> Scrollable<'a, Message> {
    scrollable(container(body).padding(Padding::ZERO.right(SCROLLBAR_GAP)))
        .width(Length::Fill)
        .height(Length::Fill)
}

const HEADER_TITLE_SIZE: u32 = 20;

/// The top of a tool page: what it is and what it does, with its main action on the right.
pub fn page_header<'a>(
    title: &'static str,
    detail: &'static str,
    action: Option<Element<'a, Message>>,
) -> Element<'a, Message> {
    let words = column![
        text(title).size(HEADER_TITLE_SIZE).font(BOLD),
        text(detail).size(EMPTY_STATE_DETAIL_SIZE).color(MUTED),
    ]
    .spacing(2);
    let mut header = row![words, space::horizontal()].align_y(Alignment::Center);
    if let Some(action) = action {
        header = header.push(action);
    }
    header.into()
}

pub const FACT_LABEL_WIDTH: f32 = 130.0;
const CARD_PADDING: f32 = 14.0;

/// A label, its value and a button that copies the value.
pub fn fact_row<'a>(label: &'static str, value: String) -> Element<'a, Message> {
    fact_row_copying(label, value.clone(), value)
}

/// Like [`fact_row`], for a value shown with a note that should not be copied with it.
pub fn fact_row_copying<'a>(
    label: &'static str,
    shown: String,
    copied: String,
) -> Element<'a, Message> {
    row![
        text(label).color(MUTED).width(FACT_LABEL_WIDTH),
        text(shown).width(Length::Fill),
        copy_button(copied, label),
    ]
    .spacing(8)
    .align_y(Alignment::Start)
    .into()
}

/// A block of facts on a raised background.
pub fn card<'a>(content: impl Into<Element<'a, Message>>) -> Element<'a, Message> {
    container(content)
        .padding(CARD_PADDING)
        .width(Length::Fill)
        .style(theme::card)
        .into()
}

const STRIP_TEXT_SIZE: u32 = 13;

/// The buttons that switch between the pages of a tab, with `selected` highlighted.
pub fn page_strip<'a, P: Copy + PartialEq + 'a>(
    pages: &[P],
    selected: P,
    title: fn(P) -> &'static str,
    on_pick: fn(P) -> Message,
) -> Element<'a, Message> {
    LayoutRow::with_children(pages.iter().map(|page| {
        button(text(title(*page)).size(STRIP_TEXT_SIZE))
            .on_press(on_pick(*page))
            .padding([5.0, 14.0])
            .style(theme::tab_button(*page == selected))
            .into()
    }))
    .spacing(8)
    .into()
}
