mod actions;
mod app;
mod export_format;
mod filter_bar;
mod host_filter;
mod host_order;
mod host_rows;
mod icons;
mod modal;
mod monitor;
mod notify;
mod opener;
mod presets;
mod results;
mod scan_flow;
mod settings;
mod settings_modal;
mod storage;
mod theme;
mod timestamp;
mod view;

use iced::{Size, window};

const DEFAULT_WINDOW: Size = Size::new(1040.0, 700.0);
const MIN_WINDOW: Size = Size::new(360.0, 480.0);

fn main() -> iced::Result {
    iced::application(app::LanPing::new, app::LanPing::update, view::view)
        .font(iced_fonts::LUCIDE_FONT_BYTES)
        .title("LanPing")
        .subscription(app::LanPing::subscription)
        .theme(app::LanPing::theme)
        .window(window::Settings {
            size: DEFAULT_WINDOW,
            min_size: Some(MIN_WINDOW),
            ..window::Settings::default()
        })
        .run()
}
