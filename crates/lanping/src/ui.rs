//! Small widgets that several pages build the same way.

use iced::widget::{Scrollable, column, container, scrollable, text};
use iced::{Alignment, Element, Length, Padding};

use crate::app::Message;
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
