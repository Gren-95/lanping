//! The per-band channel chart on the Wi-Fi tab.

use iced::widget::{Row as LayoutRow, column, progress_bar, row, text};
use iced::{Alignment, Element};
use lanping_core::wifi::{AccessPoint, Band, ChannelLoad, channel_loads, recommend_channel};

use crate::app::Message;
use crate::results::pill;
use crate::theme::{self, BOLD, GOOD, MUTED};

const BAND_SPACING: f32 = 28.0;
const BAND_ROW_SPACING: f32 = 14.0;
const CHANNEL_LABEL_WIDTH: f32 = 64.0;
const CHANNEL_BAR_WIDTH: f32 = 220.0;

/// How crowded each channel is, from every network in range. Filters do not apply here.
pub fn channel_section(access_points: &[AccessPoint]) -> Element<'_, Message> {
    let loads = channel_loads(access_points);
    let busiest = loads
        .iter()
        .map(|load| load.access_points)
        .max()
        .unwrap_or(1);
    let bands = Band::ALL.into_iter().filter_map(|band| {
        let in_band: Vec<&ChannelLoad> = loads.iter().filter(|load| load.band == band).collect();
        (!in_band.is_empty()).then(|| band_block(access_points, band, &in_band, busiest))
    });
    column![
        text("Channels").size(16).font(BOLD),
        LayoutRow::with_children(bands)
            .spacing(BAND_SPACING)
            .wrap()
            .vertical_spacing(BAND_ROW_SPACING),
    ]
    .spacing(8)
    .into()
}

fn band_block<'a>(
    access_points: &[AccessPoint],
    band: Band,
    loads: &[&ChannelLoad],
    busiest: usize,
) -> Element<'a, Message> {
    let mut title = row![text(band.to_string()).font(BOLD)]
        .spacing(8)
        .align_y(Alignment::Center);
    if let Some(channel) = recommend_channel(access_points, band) {
        title = title.push(pill(format!("Best: channel {channel}"), GOOD));
    }
    let bars = loads.iter().fold(column![].spacing(4), |list, load| {
        list.push(channel_bar(load, busiest))
    });
    column![title, bars].spacing(8).into()
}

fn channel_bar<'a>(load: &ChannelLoad, busiest: usize) -> Element<'a, Message> {
    let noun = if load.access_points == 1 {
        "network"
    } else {
        "networks"
    };
    row![
        text(format!("Ch {}", load.channel))
            .size(13)
            .width(CHANNEL_LABEL_WIDTH),
        progress_bar(0.0..=busiest as f32, load.access_points as f32)
            .girth(8)
            .length(CHANNEL_BAR_WIDTH)
            .style(theme::progress),
        text(format!(
            "{} {noun}, strongest {}%",
            load.access_points, load.strongest_percent
        ))
        .size(12)
        .color(MUTED),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}
