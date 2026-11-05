// Release builds on Windows open no console window next to the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod app;
mod copy_text;
mod dns_tab;
mod export_format;
mod filter_bar;
mod host_details;
mod host_filter;
mod host_order;
mod host_rows;
mod icons;
mod latency_tab;
mod map_dialog;
mod modal;
mod monitor;
mod notify;
mod opener;
mod presets;
mod results;
mod scan_flow;
mod settings;
mod settings_page;
mod sparkline;
mod storage;
mod subnet_tab;
mod summary_dialog;
mod table_grid;
mod tabs;
mod theme;
mod timestamp;
mod trace_tab;
mod view;
mod whois_page;
mod wifi_channels;
mod wifi_list;
mod wifi_tab;
mod wifi_table;

use iced::{Size, window};

const DEFAULT_WINDOW: Size = Size::new(1040.0, 700.0);
const MIN_WINDOW: Size = Size::new(360.0, 480.0);
/// Matches the .desktop file name so launchers show the right icon and group windows.
#[cfg(target_os = "linux")]
const APPLICATION_ID: &str = "lanping";

const WINDOW_ICON_SIZE: u32 = 64;
const WINDOW_ICON_RGBA: &[u8] = include_bytes!("../assets/window-icon-64.rgba");

fn window_icon() -> Option<window::Icon> {
    window::icon::from_rgba(
        WINDOW_ICON_RGBA.to_vec(),
        WINDOW_ICON_SIZE,
        WINDOW_ICON_SIZE,
    )
    .ok()
}

/// Saved size from the last session, kept at least as large as the minimum.
fn starting_size() -> Size {
    let (settings, _) = app::load_settings();
    settings
        .window_size
        .map_or(DEFAULT_WINDOW, |(width, height)| {
            Size::new(
                (width as f32).max(MIN_WINDOW.width),
                (height as f32).max(MIN_WINDOW.height),
            )
        })
}

fn main() -> iced::Result {
    iced::application(app::LanPing::new, app::LanPing::update, view::view)
        .font(iced_fonts::LUCIDE_FONT_BYTES)
        .title("LanPing")
        .subscription(app::LanPing::subscription)
        .theme(app::LanPing::theme)
        .window(window::Settings {
            size: starting_size(),
            exit_on_close_request: false,
            icon: window_icon(),
            min_size: Some(MIN_WINDOW),
            #[cfg(target_os = "linux")]
            platform_specific: window::settings::PlatformSpecific {
                application_id: APPLICATION_ID.to_owned(),
                ..window::settings::PlatformSpecific::default()
            },
            ..window::Settings::default()
        })
        .run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_window_icon_is_valid() {
        assert!(window_icon().is_some());
    }
}
