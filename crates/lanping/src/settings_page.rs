//! The settings page and its tabs.

use iced::widget::{
    Row, button, checkbox, column, container, mouse_area, pick_list, row, scrollable, space, text,
    text_input,
};
use iced::{Alignment, Element, Length, Padding};
use lanping_core::ports::{COMMON_SERVICES, PORT_LIST_SOURCE_URL, PortGroup, Service};

use crate::app::{LanPing, Message};
use crate::icons::lucide;
use crate::monitor::AutoInterval;
use crate::settings::{Column, SettingsTab};
use crate::tabs::Tab;
use crate::theme::{self, ACCENT, BAD, BOLD, ITEM_SPACING, MUTED};

const EDGE_PADDING: f32 = 20.0;
const TAB_INSET: f32 = 10.0;
const PORT_CELL_WIDTH: f32 = 200.0;

pub fn settings_panel(app: &LanPing) -> Element<'_, Message> {
    let tab = &app.settings_tab;
    let content = match tab {
        SettingsTab::CommonPorts => common_ports_tab(app),
        SettingsTab::CustomPorts => custom_ports_tab(app),
        SettingsTab::Columns => columns_tab(app),
        SettingsTab::Tabs => tabs_tab(app),
        SettingsTab::Monitoring => monitoring_tab(app),
        SettingsTab::Network => network_tab(app),
        SettingsTab::Presets => presets_tab(app),
    };
    let page = column![
        tab_strip(tab),
        scrollable(container(content).padding(EDGE_PADDING))
            .width(Length::Fill)
            .height(Length::Fill),
    ];
    // No padding, so the tab band and scrollbar reach the panel edges.
    container(page)
        .width(Length::Fill)
        .height(Length::Fill)
        .clip(true)
        .style(theme::panel)
        .into()
}

/// One row of connected tabs. The selected tab shares the panel color, so it joins
/// the page below. A window too narrow for the row scrolls it sideways.
fn tab_strip(selected: &SettingsTab) -> Element<'_, Message> {
    let tabs = [
        (SettingsTab::CommonPorts, lucide::list_checks(), "Ports"),
        (SettingsTab::CustomPorts, lucide::square_pen(), "Custom"),
        (SettingsTab::Columns, lucide::columns_three(), "Columns"),
        (SettingsTab::Tabs, lucide::layout_list(), "Tabs"),
        (SettingsTab::Monitoring, lucide::refresh_cw(), "Monitoring"),
        (SettingsTab::Network, lucide::network(), "Network"),
        (SettingsTab::Presets, lucide::bookmark(), "Presets"),
    ];
    let row = Row::with_children(tabs.into_iter().map(|(tab, icon, label)| {
        let is_selected = tab == *selected;
        button(icon_label(icon, label, 13))
            .on_press(Message::ShowSettings(tab))
            .padding([9.0, 8.0])
            .style(theme::folder_tab(is_selected))
            .into()
    }))
    .spacing(2);
    let hidden_bar = scrollable::Scrollbar::new().width(0).scroller_width(0);
    container(
        scrollable(container(row).padding(Padding::ZERO.left(TAB_INSET).right(TAB_INSET)))
            .direction(scrollable::Direction::Horizontal(hidden_bar))
            .width(Length::Fill),
    )
    .padding(Padding::ZERO.top(6.0))
    .width(Length::Fill)
    .style(theme::tab_band)
    .into()
}

fn common_ports_tab(app: &LanPing) -> Element<'_, Message> {
    let settings = &app.settings;
    let selected_count = settings.selected_ports.len();

    let bulk = Row::with_children(vec![
        small_icon_button(
            lucide::square_check_big(),
            "Select all",
            Some(Message::SelectAllPorts),
        ),
        small_icon_button(
            lucide::square(),
            "Select none",
            Some(Message::SelectNoPorts),
        ),
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

fn tabs_tab(app: &LanPing) -> Element<'_, Message> {
    column![
        text("Tick the tabs to show. Drag a row up or down to change the order. Devices always stays.")
            .size(13)
            .color(MUTED),
        tab_list(app),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

/// Every tab as a row you can drag up or down.
fn tab_list(app: &LanPing) -> Element<'_, Message> {
    let rows = app
        .settings
        .tabs
        .iter()
        .fold(column![].spacing(ITEM_SPACING), |list, setting| {
            let tab = setting.tab;
            let toggle = checkbox(setting.visible)
                .label(tab.title())
                .on_toggle_maybe(
                    (tab != Tab::Scan).then_some(move |visible| Message::TabToggled(tab, visible)),
                );
            let entry = row![
                lucide::grip_vertical().size(16).color(MUTED),
                tab.icon().size(16).color(MUTED),
                toggle,
            ]
            .spacing(10)
            .align_y(Alignment::Center);
            let card = container(entry).padding(10).width(Length::Fill).style(
                if app.tab_drag == Some(tab) {
                    theme::selected_card
                } else {
                    theme::card
                },
            );
            list.push(
                mouse_area(card)
                    .on_press(Message::TabDragStarted(tab))
                    .on_enter(Message::TabDraggedOver(tab))
                    .interaction(iced::mouse::Interaction::Grab),
            )
        });
    mouse_area(rows)
        .on_release(Message::TabDragEnded)
        .on_exit(Message::TabDragEnded)
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

/// A small button with a Lucide icon in front of its label.
fn small_icon_button(
    icon: iced::widget::Text<'static>,
    label: &'static str,
    message: Option<Message>,
) -> Element<'static, Message> {
    button(icon_label(icon, label, 13))
        .on_press_maybe(message)
        .padding([6.0, 14.0])
        .style(theme::quiet_button)
        .into()
}

/// An icon followed by a text label, sized to match.
fn icon_label(
    icon: iced::widget::Text<'static>,
    label: &'static str,
    size: u32,
) -> Element<'static, Message> {
    row![icon.size(size as f32 + 2.0), text(label).size(size)]
        .spacing(8)
        .align_y(Alignment::Center)
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
        text("Notify me after an automatic rescan when:").size(13).color(MUTED),
        checkbox(settings.notify_new_devices)
            .label("A new device appears")
            .on_toggle(Message::NotifyToggled),
        checkbox(settings.notify_ports_opened)
            .label("A known device opens a new port")
            .on_toggle(Message::NotifyPortsToggled),
        checkbox(settings.notify_named_offline)
            .label("A device you named goes offline")
            .on_toggle(Message::NotifyOfflineToggled),
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
        checkbox(app.settings.scan_on_startup)
            .label("Scan when LanPing starts")
            .on_toggle(Message::ScanOnStartupToggled),
        checkbox(app.settings.scan_on_network_change)
            .label("Scan when the main network changes")
            .on_toggle(Message::ScanOnNetworkChangeToggled),
        text("The network list is checked every 10 seconds while the second option is on.")
            .size(12)
            .color(MUTED),
        text("SNMP community").size(14),
        text_input("Empty leaves SNMP off", &app.settings.snmp_community)
            .on_input(Message::SnmpCommunityChanged)
            .secure(true)
            .padding(theme::CONTROL_PADDING)
            .size(13)
            .style(theme::input),
        text("Reads model, serial number, uptime and port count, and never changes anything. SNMP v2c sends the community unencrypted to every scanned host, so use a read-only one, and only on networks you trust.")
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
            small_icon_button(
                lucide::trash_two(),
                "Delete",
                Some(Message::DeletePreset(index))
            ),
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
