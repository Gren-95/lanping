//! Colors and widget styles shared by every view.

use crate::host_filter::{FAST_LATENCY_MS, SLOW_LATENCY_MS};
use iced::widget::{button, container, progress_bar, text_input};
use iced::{Background, Border, Color, Shadow, Theme, Vector, border, color};

pub const BACKDROP: Color = color!(0x0d1016);
pub const PANEL: Color = color!(0x151a23);
pub const PANEL_RAISED: Color = color!(0x1b2230);
/// Tint of every other table row; brighter than the raised panel so it shows on dim displays.
pub const STRIPE: Color = color!(0x232d40);
pub const INPUT: Color = color!(0x10141c);
pub const OUTLINE: Color = color!(0x283142);
pub const TEXT: Color = color!(0xe7edf5);
pub const MUTED: Color = color!(0x8a97aa);
pub const ACCENT: Color = color!(0x6c8cff);
pub const GOOD: Color = color!(0x3fd29a);
pub const WARN: Color = color!(0xf2b84b);
pub const BAD: Color = color!(0xf26d6d);

pub const PANEL_RADIUS: f32 = 14.0;
pub const CONTROL_RADIUS: f32 = 10.0;

pub fn app_theme() -> Theme {
    Theme::custom(
        "LanPing",
        iced::theme::Palette {
            background: BACKDROP,
            text: TEXT,
            primary: ACCENT,
            success: GOOD,
            warning: WARN,
            danger: BAD,
        },
    )
}

pub fn latency_color(milliseconds: f64) -> Color {
    if milliseconds < FAST_LATENCY_MS {
        GOOD
    } else if milliseconds < SLOW_LATENCY_MS {
        WARN
    } else {
        BAD
    }
}

pub fn backdrop(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BACKDROP)),
        text_color: Some(TEXT),
        ..container::Style::default()
    }
}

pub fn panel(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(PANEL)),
        border: Border {
            color: OUTLINE,
            width: 1.0,
            radius: PANEL_RADIUS.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: Vector::new(0.0, 6.0),
            blur_radius: 18.0,
        },
        ..container::Style::default()
    }
}

/// Background of one table line; every other line is tinted.
pub fn stripe(tinted: bool) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: tinted.then_some(Background::Color(Color {
            a: 0.55,
            ..PANEL_RAISED
        })),
        border: border::rounded(8),
        ..container::Style::default()
    }
}

pub fn card(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(PANEL_RAISED)),
        border: Border {
            color: OUTLINE,
            width: 1.0,
            radius: 12.0.into(),
        },
        ..container::Style::default()
    }
}

/// A card marked as the one being moved or looked at.
pub fn selected_card(theme: &Theme) -> container::Style {
    container::Style {
        border: Border {
            color: ACCENT,
            ..card(theme).border
        },
        ..card(theme)
    }
}

/// A pill tinted with `tint`.
pub fn chip(tint: Color) -> impl Fn(&Theme) -> container::Style {
    move |_theme| container::Style {
        background: Some(Background::Color(Color { a: 0.16, ..tint })),
        text_color: Some(tint),
        border: border::rounded(999),
        ..container::Style::default()
    }
}

pub fn primary_button(theme: &Theme, status: button::Status) -> button::Style {
    rounded_button(button::primary(theme, status))
}

pub fn danger_button(theme: &Theme, status: button::Status) -> button::Style {
    rounded_button(button::danger(theme, status))
}

pub fn quiet_button(_theme: &Theme, status: button::Status) -> button::Style {
    let (background, text_color) = match status {
        button::Status::Hovered => (PANEL_RAISED, TEXT),
        button::Status::Pressed => (OUTLINE, TEXT),
        button::Status::Disabled => (INPUT, Color { a: 0.4, ..MUTED }),
        button::Status::Active => (INPUT, MUTED),
    };
    button::Style {
        background: Some(Background::Color(background)),
        text_color,
        border: Border {
            color: OUTLINE,
            width: 1.0,
            radius: CONTROL_RADIUS.into(),
        },
        ..button::Style::default()
    }
}

/// Table header buttons: plain text that brightens on hover.
pub fn header_button(_theme: &Theme, status: button::Status) -> button::Style {
    let text_color = match status {
        button::Status::Hovered | button::Status::Pressed => TEXT,
        _ => MUTED,
    };
    button::Style {
        text_color,
        ..button::Style::default()
    }
}

fn rounded_button(base: button::Style) -> button::Style {
    button::Style {
        border: border::rounded(CONTROL_RADIUS),
        ..base
    }
}

/// Where a control sits in a group of joined controls. Only the outer corners of
/// the group are rounded, so neighbors read as one input group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Alone,
    First,
    Middle,
    Last,
}

impl Edge {
    fn radius(self) -> border::Radius {
        match self {
            Self::Alone => CONTROL_RADIUS.into(),
            Self::First => border::left(CONTROL_RADIUS),
            Self::Middle => 0.0.into(),
            Self::Last => border::right(CONTROL_RADIUS),
        }
    }
}

pub fn input_at(edge: Edge) -> impl Fn(&Theme, text_input::Status) -> text_input::Style {
    move |theme, status| {
        let mut style = input(theme, status);
        style.border.radius = edge.radius();
        style
    }
}

pub fn pick_at(
    edge: Edge,
) -> impl Fn(&Theme, iced::widget::pick_list::Status) -> iced::widget::pick_list::Style {
    move |theme, status| {
        let mut style = pick(theme, status);
        style.border.radius = edge.radius();
        style
    }
}

/// Re-rounds a button style to fit `edge`.
pub fn button_at(
    edge: Edge,
    base: impl Fn(&Theme, button::Status) -> button::Style,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let mut style = base(theme, status);
        style.border.radius = edge.radius();
        style
    }
}

/// Padding shared by inputs and dropdowns so a toolbar row has one height.
pub const CONTROL_PADDING: [f32; 2] = [8.0, 10.0];

pub fn input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let base = text_input::default(theme, status);
    let outline = match status {
        text_input::Status::Focused { .. } => ACCENT,
        text_input::Status::Hovered => MUTED,
        _ => OUTLINE,
    };
    text_input::Style {
        background: Background::Color(INPUT),
        border: Border {
            color: outline,
            width: 1.0,
            radius: CONTROL_RADIUS.into(),
        },
        placeholder: Color { a: 0.55, ..MUTED },
        ..base
    }
}

pub fn progress(_theme: &Theme) -> progress_bar::Style {
    progress_bar::Style {
        background: Background::Color(INPUT),
        bar: Background::Color(ACCENT),
        border: border::rounded(999),
    }
}

pub fn pick(
    theme: &Theme,
    status: iced::widget::pick_list::Status,
) -> iced::widget::pick_list::Style {
    let base = iced::widget::pick_list::default(theme, status);
    iced::widget::pick_list::Style {
        background: Background::Color(INPUT),
        border: Border {
            color: OUTLINE,
            width: 1.0,
            radius: CONTROL_RADIUS.into(),
        },
        placeholder_color: Color { a: 0.55, ..MUTED },
        ..base
    }
}

pub fn scrim(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgba(0.0, 0.0, 0.0, 0.62))),
        ..container::Style::default()
    }
}

pub fn dialog(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(PANEL)),
        border: Border {
            color: OUTLINE,
            width: 1.0,
            radius: PANEL_RADIUS.into(),
        },
        shadow: Shadow {
            color: Color::from_rgba(0.0, 0.0, 0.0, 0.6),
            offset: Vector::new(0.0, 12.0),
            blur_radius: 36.0,
        },
        ..container::Style::default()
    }
}

/// Tab strip button; the selected tab is filled with the accent color.
pub fn tab_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        if selected {
            return primary_button(theme, button::Status::Active);
        }
        quiet_button(theme, status)
    }
}
