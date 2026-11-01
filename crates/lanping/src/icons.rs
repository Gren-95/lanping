//! Icon buttons: Lucide glyphs with a tooltip that names the action.

use iced::widget::{Button, button, container, text, tooltip};
use iced::{Element, Padding};

use crate::app::Message;
use crate::theme::{self, Edge};

pub use iced_fonts::lucide;

/// Icon size in the toolbar buttons.
pub const ICON_SIZE: f32 = 16.0;
/// Gives an icon button the same height as an input or dropdown.
const ICON_BUTTON_PADDING: [f32; 2] = [6.0, 10.0];

/// A quiet button around `content`, joined to its neighbors as `edge` says.
/// Pass `None` to disable it.
pub fn icon_button<'a>(
    content: impl Into<Element<'a, Message>>,
    on_press: Option<Message>,
    edge: Edge,
) -> Button<'a, Message> {
    button(content)
        .on_press_maybe(on_press)
        .padding(Padding::from(ICON_BUTTON_PADDING))
        .style(theme::button_at(edge, theme::quiet_button))
}

/// Shows `label` when the pointer rests on `target`.
pub fn with_tooltip<'a>(
    target: impl Into<Element<'a, Message>>,
    label: &'static str,
) -> Element<'a, Message> {
    tooltip(
        target,
        container(text(label).size(12)).padding([4.0, 8.0]),
        tooltip::Position::Bottom,
    )
    .style(container::rounded_box)
    .gap(4)
    .into()
}
