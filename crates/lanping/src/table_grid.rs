//! One line of a table: cells with fixed widths, so every row lines up with the header.

use iced::widget::{Row as LayoutRow, container};
use iced::{Alignment, Element, Length};

use crate::app::Message;

const CELL_PADDING: f32 = 8.0;
const CELL_SIDE_PADDING: f32 = 12.0;

/// Horizontal room a column of `width` takes, including the cell padding.
pub const fn cell_width(content_width: f32) -> f32 {
    content_width + 2.0 * CELL_SIDE_PADDING
}

pub fn grid_line<'a>(cells: Vec<(f32, Element<'a, Message>)>) -> LayoutRow<'a, Message> {
    LayoutRow::with_children(cells.into_iter().map(|(width, cell)| {
        container(cell)
            .width(Length::Fixed(width))
            .padding([CELL_PADDING, CELL_SIDE_PADDING])
            .into()
    }))
    .align_y(Alignment::Center)
}
