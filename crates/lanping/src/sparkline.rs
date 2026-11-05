//! A row of small bars, one per recent sample, shared by the Wi-Fi and trace tables.

use iced::widget::{Row as LayoutRow, container, space};
use iced::{Alignment, Background, Color, Element, Theme, border};

use crate::app::Message;

const HEIGHT: f32 = 24.0;
const BAR_WIDTH: f32 = 4.0;
const BAR_SPACING: f32 = 1.0;
/// Height of the dash that stands in for a sample with no value.
const GAP_HEIGHT: f32 = 2.0;

/// One sample: how tall its bar is, as a share of the full height, and its color.
/// `None` draws the dash for a missing sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bar {
    pub fraction: Option<f32>,
    pub color: Color,
}

/// Width of a sparkline with `count` bars.
pub fn width_of(count: usize) -> f32 {
    let count = count as f32;
    count * BAR_WIDTH + (count - 1.0).max(0.0) * BAR_SPACING
}

pub fn sparkline<'a>(bars: impl IntoIterator<Item = Bar>) -> Element<'a, Message> {
    chart(bars, HEIGHT)
}

/// The same bars drawn `height` tall, for a graph that stands on its own.
pub fn chart<'a>(bars: impl IntoIterator<Item = Bar>, height: f32) -> Element<'a, Message> {
    let bars = bars.into_iter().map(|bar| {
        let bar_height = bar
            .fraction
            .map_or(GAP_HEIGHT, |fraction| (height * fraction).max(GAP_HEIGHT));
        container(space::horizontal())
            .width(BAR_WIDTH)
            .height(bar_height)
            .style(bar_style(bar.color))
            .into()
    });
    let bars = LayoutRow::with_children(bars)
        .spacing(BAR_SPACING)
        .align_y(Alignment::End);
    container(bars).align_bottom(height).into()
}

fn bar_style(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(color)),
        border: border::rounded(1),
        ..container::Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn width_counts_bars_and_the_gaps_between_them() {
        assert_eq!(width_of(0), 0.0);
        assert_eq!(width_of(1), BAR_WIDTH);
        assert_eq!(width_of(3), 3.0 * BAR_WIDTH + 2.0 * BAR_SPACING);
    }
}
