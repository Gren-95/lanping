//! The settings dialog and its tabs.

use iced::font::Weight;
use iced::widget::{
    Row, button, checkbox, column, container, mouse_area, pick_list, row, scrollable, space, text,
    text_input,
};
use iced::{Alignment, Element, Font, Length};
use lanping_core::ports::{COMMON_SERVICES, PORT_LIST_SOURCE_URL, PortGroup, Service};

use crate::app::{LanPing, Message};
use crate::icons::lucide;
use crate::monitor::AutoInterval;
use crate::settings::{Column, SettingsTab};
use crate::theme::{self, ACCENT, BAD, MUTED};

const ITEM_SPACING: f32 = 10.0;
const PORT_CELL_WIDTH: f32 = 200.0;
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};

pub fn settings_dialog<'a>(app: &'a LanPing, tab: &'a SettingsTab) -> Element<'a, Message> {
    let header = row![
        text("Settings").size(22).font(BOLD),
        space::horizontal(),
        button(text("Done").size(14))
            .on_press(Message::CloseModal)
            .padding([6.0, 16.0])
            .style(theme::primary_button),
    ]
    .align_y(Alignment::Center);

    let content = match tab {
        SettingsTab::CommonPorts => common_ports_tab(app),
        SettingsTab::CustomPorts => custom_ports_tab(app),
        SettingsTab::Columns => columns_tab(app),
        SettingsTab::Monitoring => monitoring_tab(app),
        SettingsTab::Network => network_tab(app),
        SettingsTab::Presets => presets_tab(app),
    };

    column![
        header,
        tab_strip(tab),
        scrollable(container(content).padding(iced::Padding::ZERO.right(12.0)))
            .height(Length::Fill),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn tab_strip(selected: &SettingsTab) -> Element<'_, Message> {
    let tabs = [
        (SettingsTab::CommonPorts, "Common ports"),
        (SettingsTab::CustomPorts, "Custom ports"),
        (SettingsTab::Columns, "Columns"),
        (SettingsTab::Monitoring, "Monitoring"),
        (SettingsTab::Network, "Network"),
        (SettingsTab::Presets, "Presets"),
    ];
    Row::with_children(tabs.into_iter().map(|(tab, label)| {
        let is_selected = tab == *selected;
        button(text(label).size(14))
            .on_press(Message::ShowSettings(tab))
            .padding([7.0, 16.0])
            .style(theme::tab_button(is_selected))
            .into()
    }))
    .spacing(8)
    .wrap()
    .vertical_spacing(8)
    .into()
}

fn common_ports_tab(app: &LanPing) -> Element<'_, Message> {
    let settings = &app.settings;
    let selected_count = settings.selected_ports.len();

    let bulk = Row::with_children(vec![
        small_button("Select all", Some(Message::SelectAllPorts)),
        small_button("Select none", Some(Message::SelectNoPorts)),
        text(format!(
            "{selected_count} of {} selected",
            COMMON_SERVICES.len()
        ))
        .size(13)
        .color(MUTED)
        .into(),
    ])
    .spacing(ITEM_SPACING)
    .align_y(Alignment::Center)
    .wrap()
    .vertical_spacing(8);

    let groups = PortGroup::ALL
        .into_iter()
        .fold(column![].spacing(ITEM_SPACING + 6.0), |list, group| {
            list.push(port_group(app, group))
        });

    column![
        text("Hosts that answer are checked for the ticked ports. Untick all ports and clear the custom list to skip port scanning.")
            .size(13)
            .color(MUTED),
        checkbox(settings.grab_banners)
            .label("Read service banners (SSH version, web server, and similar)")
            .on_toggle(Message::BannersToggled),
        bulk,
        groups,
        source_link(),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn port_group(app: &LanPing, group: PortGroup) -> Element<'_, Message> {
    let cells = COMMON_SERVICES
        .iter()
        .filter(|service| service.group == group)
        .map(|service| port_cell(app, service));
    column![
        text(group.title()).size(13).font(BOLD).color(ACCENT),
        Row::with_children(cells)
            .spacing(ITEM_SPACING)
            .wrap()
            .vertical_spacing(10),
    ]
    .spacing(8)
    .into()
}

fn port_cell<'a>(app: &LanPing, service: &'a Service) -> Element<'a, Message> {
    let port = service.port;
    let ticked = app.settings.selected_ports.contains(&port);
    container(
        checkbox(ticked)
            .label(format!("{port}  {}", service.name))
            .on_toggle(move |selected| Message::PortToggled(port, selected)),
    )
    .width(PORT_CELL_WIDTH)
    .into()
}

fn source_link<'a>() -> Element<'a, Message> {
    row![
        text("Port numbers and names from").size(12).color(MUTED),
        button(text("Wikipedia").size(12))
            .on_press(Message::OpenUrl(PORT_LIST_SOURCE_URL.to_owned()))
            .padding([2.0, 8.0])
            .style(theme::quiet_button),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

fn custom_ports_tab(app: &LanPing) -> Element<'_, Message> {
    let input = text_input("22, 80, 8000-8010", &app.settings.custom_ports)
        .on_input(Message::CustomPortsChanged)
        .padding(10)
        .style(theme::input);
    let problem: Element<'_, Message> = match app.settings.resolve_ports() {
        Ok(ports) => text(format!("{} ports will be scanned in total", ports.len()))
            .size(13)
            .color(MUTED)
            .into(),
        Err(error) => text(error.to_string()).size(13).color(BAD).into(),
    };
    column![
        text("Extra ports are scanned together with the ticked common ports.")
            .size(13)
            .color(MUTED),
        input,
        text("Separate ports with commas. Use a dash for ranges.")
            .size(12)
            .color(MUTED),
        problem,
    ]
    .spacing(ITEM_SPACING)
    .into()
}

fn columns_tab(app: &LanPing) -> Element<'_, Message> {
    column![
        text("Tick the columns to show. Drag a row up or down to change the order. Address always stays.")
            .size(13)
            .color(MUTED),
        column_list(app, None),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

/// Every column as a row you can drag up or down; `highlight` marks one row.
pub fn column_list(app: &LanPing, highlight: Option<Column>) -> Element<'_, Message> {
    let rows =
        app.settings
            .columns
            .iter()
            .fold(column![].spacing(ITEM_SPACING), |list, setting| {
                let column = setting.column;
                let can_hide = column != Column::Address;
                let toggle = checkbox(setting.visible)
                    .label(column.title())
                    .on_toggle_maybe(
                        can_hide.then_some(move |visible| Message::ColumnToggled(column, visible)),
                    );
                let entry = row![lucide::grip_vertical().size(16).color(MUTED), toggle]
                    .spacing(10)
                    .align_y(Alignment::Center);
                let emphasized = app.column_drag == Some(column) || highlight == Some(column);
                let card = container(entry)
                    .padding(10)
                    .width(Length::Fill)
                    .style(if emphasized {
                        theme::selected_card
                    } else {
                        theme::card
                    });
                list.push(
                    mouse_area(card)
                        .on_press(Message::ColumnDragStarted(column))
                        .on_enter(Message::ColumnDraggedOver(column))
                        .interaction(iced::mouse::Interaction::Grab),
                )
            });
    mouse_area(rows)
        .on_release(Message::ColumnDragEnded)
        .on_exit(Message::ColumnDragEnded)
        .into()
}

pub fn small_button(label: &'static str, message: Option<Message>) -> Element<'static, Message> {
    button(text(label).size(13))
        .on_press_maybe(message)
        .padding([6.0, 14.0])
        .style(theme::quiet_button)
        .into()
}

fn monitoring_tab(app: &LanPing) -> Element<'_, Message> {
    let settings = &app.settings;
    let interval = pick_list(
        &AutoInterval::ALL[..],
        Some(AutoInterval(settings.auto_rescan_minutes)),
        Message::AutoRescanPicked,
    )
    .text_size(14)
    .width(220)
    .style(theme::pick);
    column![
        text("Rescan the network on a timer while LanPing is open. Results are compared with the previous scan of the same network.")
            .size(13)
            .color(MUTED),
        row![text("Automatic rescan").size(14), interval]
            .spacing(ITEM_SPACING)
            .align_y(Alignment::Center),
        checkbox(settings.notify_new_devices)
            .label("Show a desktop notification when a new device appears")
            .on_toggle(Message::NotifyToggled),
        text("Notifications use notify-send on Linux and osascript on macOS. Windows shows results in the app only.")
            .size(12)
            .color(MUTED),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn network_tab(app: &LanPing) -> Element<'_, Message> {
    column![
        checkbox(app.settings.ignore_virtual_networks)
            .label("Ignore virtual adapters")
            .on_toggle(Message::IgnoreVirtualToggled),
        text("Hides container bridges, VPN tunnels and VM switches from the network list. Tailscale devices stay listed.")
            .size(12)
            .color(MUTED),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn presets_tab(app: &LanPing) -> Element<'_, Message> {
    let saved = &app.settings.filter_presets;
    let mut content = column![
        text("Presets are saved with the Save preset button next to the filters. Built-in presets cannot be removed.")
            .size(13)
            .color(MUTED),
    ]
    .spacing(ITEM_SPACING + 4.0);

    if saved.is_empty() {
        content = content.push(text("No saved presets yet.").size(14));
    }
    for (index, preset) in saved.iter().enumerate() {
        let entry = row![
            text(preset.name.as_str()).size(14).font(BOLD),
            text(format!("{} filters", preset.filter.active_count()))
                .size(12)
                .color(MUTED),
            space::horizontal(),
            small_button("Delete", Some(Message::DeletePreset(index))),
        ]
        .spacing(ITEM_SPACING)
        .align_y(Alignment::Center);
        content = content.push(
            container(entry)
                .padding(10)
                .width(Length::Fill)
                .style(theme::card),
        );
    }
    content.into()
}
