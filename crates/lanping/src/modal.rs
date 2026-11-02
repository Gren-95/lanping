//! Modal dialogs: settings and the per-host "Open with" menu.

use iced::font::Weight;
use iced::widget::{
    Text, button, column, container, mouse_area, opaque, row, scrollable, space, text, text_input,
};
use iced::{Alignment, Element, Font, Length};
use lanping_core::Host;

use crate::actions::{HostAction, grouped_actions};
use crate::app::{LanPing, Message, Modal};
use crate::host_details::host_details_dialog;
use crate::icons::{ICON_SIZE, lucide};
use crate::map_dialog::map_dialog;
use crate::results::pill;
use crate::settings::Column;
use crate::settings_modal::{column_list, settings_dialog, small_button};
use crate::summary_dialog::summary_dialog;
use crate::theme::{self, ACCENT, MUTED};

const DIALOG_MAX_WIDTH: f32 = 600.0;
const DIALOG_MAX_HEIGHT: f32 = 620.0;
const DIAGNOSTIC_HEIGHT: f32 = 320.0;
const DIALOG_PADDING: f32 = 20.0;
const DIALOG_MARGIN: f32 = 16.0;
const ITEM_SPACING: f32 = 10.0;
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};

/// Dims the page and shows `modal` centered on top. Clicking outside closes it.
pub fn overlay<'a>(app: &'a LanPing, modal: &'a Modal) -> Element<'a, Message> {
    let (dialog, fill_height) = match modal {
        Modal::Settings(tab) => (settings_dialog(app, tab), true),
        Modal::HostActions(key) => match app.find_host(key) {
            Some(host) => (host_actions_dialog(app, host), false),
            None => return space::vertical().height(0).into(),
        },
        Modal::Diagnostic { title, output } => (diagnostic_dialog(title, output.as_deref()), false),
        Modal::HostDetails(key) => (host_details_dialog(app, key), true),
        Modal::Summary(summary) => (summary_dialog(summary), false),
        Modal::NetworkMap(gateway) => (map_dialog(app, *gateway), true),
        Modal::SavePreset => (save_preset_dialog(app), false),
        Modal::ColumnMenu(column) => (column_menu_dialog(app, *column), false),
    };
    // Settings lays out its own padding so its tabs and scrollbar reach the edges.
    let padding = if matches!(modal, Modal::Settings(_)) {
        0.0
    } else {
        DIALOG_PADDING
    };
    let mut frame = container(dialog)
        .padding(padding)
        .width(Length::Fill)
        .max_width(DIALOG_MAX_WIDTH)
        .max_height(DIALOG_MAX_HEIGHT)
        .style(theme::dialog);
    if fill_height {
        frame = frame.height(Length::Fill);
    }
    let layer = container(opaque(frame))
        .width(Length::Fill)
        .height(Length::Fill)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(DIALOG_MARGIN)
        .style(theme::scrim);
    opaque(mouse_area(layer).on_press(Message::CloseModal))
}

fn host_actions_dialog<'a>(app: &'a LanPing, host: &'a Host) -> Element<'a, Message> {
    let key = host.device_key();
    let subtitle = app
        .labels
        .get(&key)
        .map(str::to_owned)
        .or_else(|| host.display_name().map(str::to_owned))
        .or_else(|| host.vendor.clone())
        .unwrap_or_else(|| "Unknown device".to_owned());
    let header = row![
        column![
            text(host.ip.to_string()).size(22).font(BOLD),
            text(subtitle).size(13).color(MUTED),
        ]
        .spacing(2),
        space::horizontal(),
        button(text("Close").size(14))
            .on_press(Message::CloseModal)
            .padding([6.0, 16.0])
            .style(theme::quiet_button),
    ]
    .align_y(Alignment::Start);

    let mut body = column![label_editor(app, &key)].spacing(ITEM_SPACING + 4.0);
    if !host.banners.is_empty() {
        body = body.push(services_list(host));
    }
    for (group, actions) in grouped_actions(host) {
        let entries = actions
            .into_iter()
            .fold(column![].spacing(8), |list, action| {
                list.push(action_button(host, action))
            });
        body = body.push(column![text(group.title()).size(13).color(MUTED), entries].spacing(8));
    }

    column![
        header,
        scrollable(container(body).padding(iced::Padding::ZERO.right(12.0))).height(Length::Shrink),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn label_editor<'a>(app: &'a LanPing, key: &str) -> Element<'a, Message> {
    let input = text_input(
        "Name this device, for example Living room TV",
        &app.label_draft,
    )
    .on_input(Message::LabelDraftChanged)
    .on_submit(Message::SaveLabel(key.to_owned()))
    .padding(10)
    .style(theme::input);
    column![
        text("Device name").size(13).color(MUTED),
        row![
            input,
            small_button("Save", Some(Message::SaveLabel(key.to_owned())))
        ]
        .spacing(ITEM_SPACING)
        .align_y(Alignment::Center),
        text("Names are stored by MAC address, so they follow the device between scans.")
            .size(12)
            .color(MUTED),
    ]
    .spacing(6)
    .into()
}

fn services_list(host: &Host) -> Element<'_, Message> {
    let rows = host
        .banners
        .iter()
        .fold(column![].spacing(6), |list, banner| {
            list.push(
                row![
                    pill(banner.port.to_string(), ACCENT),
                    text(banner.text.as_str()).size(13).font(Font::MONOSPACE),
                ]
                .spacing(ITEM_SPACING)
                .align_y(Alignment::Center),
            )
        });
    column![text("Services").size(13).color(MUTED), rows]
        .spacing(6)
        .into()
}

/// Output of a ping or traceroute, with a copy button once it has finished.
fn diagnostic_dialog<'a>(title: &'a str, output: Option<&'a str>) -> Element<'a, Message> {
    let body: Element<'a, Message> = match output {
        None => text("Running...").size(13).color(MUTED).into(),
        Some(output) => scrollable(text(output).font(Font::MONOSPACE).size(13))
            .height(Length::Fixed(DIAGNOSTIC_HEIGHT))
            .width(Length::Fill)
            .into(),
    };
    let copy = small_button(
        "Copy output",
        output.map(|output| Message::CopyText(output.to_owned(), "output")),
    );
    column![
        text(title).size(22).font(BOLD),
        body,
        row![
            space::horizontal(),
            copy,
            small_button("Close", Some(Message::CloseModal))
        ]
        .spacing(ITEM_SPACING),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

/// Right-click menu of a table column: the column list, with the clicked one marked.
fn column_menu_dialog(app: &LanPing, column: Column) -> Element<'_, Message> {
    column![
        text("Columns").size(22).font(BOLD),
        text(format!(
            "Drag a row to reorder, untick to hide. You opened this from the {} column.",
            column.title()
        ))
        .size(13)
        .color(MUTED),
        column_list(app, Some(column)),
        row![
            space::horizontal(),
            small_button("Done", Some(Message::CloseModal))
        ],
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn save_preset_dialog(app: &LanPing) -> Element<'_, Message> {
    let input = text_input("Preset name, for example Printers", &app.preset_name_draft)
        .on_input(Message::PresetNameChanged)
        .on_submit(Message::SavePreset)
        .padding(10)
        .style(theme::input);
    column![
        text("Save filters as preset").size(22).font(BOLD),
        text(format!(
            "Saves the {} active filters so you can apply them again with one click.",
            app.filter.active_count()
        ))
        .size(13)
        .color(MUTED),
        input,
        row![
            small_button("Cancel", Some(Message::CloseModal)),
            small_button("Save preset", Some(Message::SavePreset)),
        ]
        .spacing(ITEM_SPACING),
    ]
    .spacing(ITEM_SPACING + 4.0)
    .into()
}

fn action_icon(action: &HostAction) -> Text<'static> {
    match action {
        HostAction::OpenWeb { https: false, .. } => lucide::globe(),
        HostAction::OpenWeb { https: true, .. } => lucide::lock(),
        HostAction::BrowseFiles => lucide::folder_open(),
        HostAction::Ping => lucide::activity(),
        HostAction::Traceroute => lucide::route(),
        HostAction::WakeOnLan => lucide::power(),
        HostAction::CopySshCommand => lucide::terminal(),
        HostAction::CopyAddress => lucide::hash(),
        HostAction::CopyMac => lucide::fingerprint(),
        HostAction::CopyHostname => lucide::tag(),
    }
}

fn action_button(host: &Host, action: HostAction) -> Element<'_, Message> {
    let port_is_open = action
        .matching_port()
        .is_some_and(|port| host.open_ports.contains(&port));
    let mut content = row![
        action_icon(&action).size(ICON_SIZE).color(MUTED),
        text(action.label()).size(14),
        space::horizontal()
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    if port_is_open {
        content = content.push(pill("port open", ACCENT));
    }
    button(content)
        .on_press(Message::RunHostAction(host.device_key(), action))
        .padding([10.0, 14.0])
        .width(Length::Fill)
        .style(theme::quiet_button)
        .into()
}
