use iced::font::Weight;
use iced::widget::{
    Column, Row, button, column, container, pick_list, progress_bar, responsive, row, space, stack,
    text, text_input,
};
use iced::{Alignment, Element, Font, Length};

use crate::app::{HistoryEntry, LanPing, Message, Notice};
use crate::export_format::ExportFormat;
use crate::filter_bar::filter_bar;
use crate::host_order::SortKey;
use crate::icons::{ICON_SIZE, icon_button, lucide, with_tooltip};
use crate::modal::overlay;
use crate::monitor::AutoInterval;
use crate::results::{pill, results};
use crate::settings::SettingsTab;
use crate::theme::Edge;
use crate::theme::{self, ACCENT, BAD, GOOD, MUTED, TEXT};
use lanping_core::ScanStage;
use lanping_core::diff::HostStatus;
use lanping_core::range::LocalNetwork;

/// Windows at least this wide put the scan controls beside the title.
/// Windows at least this wide lay each toolbar out as one aligned row.
const TOOLBAR_ROW_WIDTH: f32 = 820.0;
const SPLIT_HEADER_WIDTH: f32 = 760.0;
const HEADER_WIDTH: f32 = 300.0;
const CONTROLS_WIDTH: f32 = 380.0;
const CONTROL_SPACING: f32 = 8.0;
/// Joined controls overlap by their border width.
const GROUP_SPACING: f32 = -1.0;
const PAGE_PADDING: f32 = 18.0;
const PANEL_PADDING: f32 = 16.0;
const SECTION_SPACING: f32 = 14.0;
pub(crate) const ITEM_SPACING: f32 = 10.0;
const BOLD: Font = Font {
    weight: Weight::Bold,
    ..Font::DEFAULT
};

pub fn view(app: &LanPing) -> Element<'_, Message> {
    let page = responsive(move |size| layout(app, size.width));
    let base = container(page)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::backdrop);
    match &app.modal {
        Some(modal) => stack![base, overlay(app, modal)].into(),
        None => base.into(),
    }
}

fn layout(app: &LanPing, width: f32) -> Element<'_, Message> {
    let top: Element<'_, Message> = if width >= SPLIT_HEADER_WIDTH {
        row![
            container(header(app)).width(HEADER_WIDTH),
            space::horizontal(),
            container(scan_controls(app)).width(CONTROLS_WIDTH),
        ]
        .spacing(SECTION_SPACING)
        .into()
    } else {
        column![header(app), panel(scan_controls(app))]
            .spacing(SECTION_SPACING)
            .into()
    };
    column![top, results_panel(app, width), status_line(app)]
        .spacing(SECTION_SPACING)
        .padding(PAGE_PADDING)
        .into()
}

fn header(app: &LanPing) -> Element<'_, Message> {
    let title = column![
        text("LanPing").size(28).font(BOLD),
        text("See every device on your network")
            .size(14)
            .color(MUTED),
    ]
    .spacing(2);

    let host_count = if app.filter.active_count() > 0 {
        format!("{} of {} hosts", app.visible_rows().len(), app.hosts.len())
    } else {
        format!("{} hosts", app.hosts.len())
    };
    let mut stats: Vec<Element<'_, Message>> = vec![pill(host_count, ACCENT)];
    let new_hosts = app.diff.count(HostStatus::New);
    if new_hosts > 0 {
        stats.push(pill(format!("{new_hosts} new"), GOOD));
    }
    if !app.diff.gone.is_empty() {
        stats.push(pill(format!("{} gone", app.diff.gone.len()), BAD));
    }
    if let Some(minutes) = AutoInterval(app.settings.auto_rescan_minutes).minutes() {
        stats.push(pill(format!("Auto {minutes} min"), MUTED));
    }
    if app.scanning {
        stats.push(pill("Scanning", theme::WARN));
    }
    stats.push(with_tooltip(
        icon_button(
            lucide::settings().size(ICON_SIZE),
            Some(Message::ShowSettings(SettingsTab::CommonPorts)),
            Edge::Alone,
        ),
        "Settings",
    ));
    let stats = Row::with_children(stats)
        .spacing(8)
        .align_y(Alignment::Center)
        .wrap()
        .vertical_spacing(8);

    column![title, stats].spacing(ITEM_SPACING).into()
}

/// Scan target, network suggestions and progress. Unframed: the caller decides the backdrop.
fn scan_controls(app: &LanPing) -> Element<'_, Message> {
    let mut content = column![target_row(app)].spacing(CONTROL_SPACING);
    let suggestions = app.network_suggestions();
    if !suggestions.is_empty() {
        content = content.push(network_suggestions(suggestions));
    }
    if app.progress.1 > 0 {
        content = content.push(progress_row(app));
    }
    content.into()
}

fn target_row(app: &LanPing) -> Element<'_, Message> {
    let mut target = text_input("192.168.0.0/24 or 10.0.0.1-50", &app.target)
        .padding(9)
        .size(15)
        .style(theme::input_at(Edge::First));
    if !app.scanning {
        target = target
            .on_input(Message::TargetChanged)
            .on_submit(Message::StartScan);
    }

    let scan_button = if app.scanning {
        with_tooltip(
            button(lucide::square().size(ICON_SIZE + 2.0))
                .on_press(Message::StopScan)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::danger_button)),
            "Stop scan",
        )
    } else {
        with_tooltip(
            button(lucide::scan_search().size(ICON_SIZE + 2.0))
                .on_press(Message::StartScan)
                .padding([7.0, 20.0])
                .style(theme::button_at(Edge::Last, theme::primary_button)),
            "Scan",
        )
    };

    input_group(vec![target.into(), scan_button])
}

/// Detected networks that match what is typed in the target field.
fn network_suggestions<'a>(networks: Vec<LocalNetwork>) -> Element<'a, Message> {
    let entries = networks.into_iter().map(|network| {
        button(
            row![
                lucide::network().size(ICON_SIZE - 2.0).color(MUTED),
                text(network.to_string()).size(13)
            ]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(Message::NetworkPicked(network))
        .width(Length::Fill)
        .padding([6.0, 12.0])
        .style(theme::quiet_button)
        .into()
    });
    Column::with_children(entries).spacing(4).into()
}

fn progress_row(app: &LanPing) -> Element<'_, Message> {
    let (scanned, total) = app.progress;
    let percent = scanned * 100 / total;
    let stage = match app.progress_stage {
        ScanStage::Sweep => "Finding hosts",
        ScanStage::Details => "Names and ports",
    };
    row![
        text(stage).size(12).color(MUTED),
        progress_bar(0.0..=total as f32, scanned as f32)
            .girth(6)
            .style(theme::progress),
        text(format!("{percent}%  ({scanned}/{total})"))
            .size(12)
            .color(MUTED),
    ]
    .spacing(ITEM_SPACING)
    .align_y(Alignment::Center)
    .into()
}

fn results_panel(app: &LanPing, width: f32) -> Element<'_, Message> {
    let content = column![toolbar(app, width >= TOOLBAR_ROW_WIDTH), results(app)]
        .spacing(ITEM_SPACING)
        .height(Length::Fill);
    container(
        container(content)
            .padding(PANEL_PADDING)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .style(theme::panel)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn toolbar(app: &LanPing, wide: bool) -> Element<'_, Message> {
    column![filter_bar(app, wide), tools_row(app, wide)]
        .spacing(8)
        .into()
}

/// Controls fused into one input group: no gap, so their borders meet.
pub(crate) fn input_group(items: Vec<Element<'_, Message>>) -> Element<'_, Message> {
    Row::with_children(items)
        .spacing(GROUP_SPACING)
        .align_y(Alignment::Center)
        .into()
}

/// Items of one toolbar line. Wide windows keep one line; narrow ones wrap.
pub(crate) fn toolbar_row(items: Vec<Element<'_, Message>>, wide: bool) -> Element<'_, Message> {
    let line = Row::with_children(items)
        .spacing(ITEM_SPACING)
        .align_y(Alignment::Center);
    if wide {
        line.into()
    } else {
        line.wrap().vertical_spacing(8).into()
    }
}

fn tools_row(app: &LanPing, wide: bool) -> Element<'_, Message> {
    let has_hosts = !app.hosts.is_empty();
    let history = pick_list(
        app.history.as_slice(),
        None::<HistoryEntry>,
        Message::HistoryPicked,
    )
    .placeholder("Saved scans")
    .text_size(13)
    .padding(theme::CONTROL_PADDING)
    .width(240)
    .style(theme::pick_at(Edge::First));
    let export_formats = if has_hosts {
        ExportFormat::ALL.to_vec()
    } else {
        Vec::new()
    };
    let export = pick_list(export_formats, None::<ExportFormat>, Message::ExportPicked)
        .placeholder("Export")
        .text_size(13)
        .padding(theme::CONTROL_PADDING)
        .width(120)
        .style(theme::pick_at(Edge::Last));

    let sort_key = pick_list(
        &SortKey::ALL[..],
        Some(app.sort_key),
        Message::SortKeyPicked,
    )
    .text_size(13)
    .padding(theme::CONTROL_PADDING)
    .width(150)
    .style(theme::pick_at(Edge::First));
    let (direction_icon, direction_tip) = if app.sort_ascending {
        (lucide::arrow_up(), "Ascending")
    } else {
        (lucide::arrow_down(), "Descending")
    };
    let direction = with_tooltip(
        icon_button(
            direction_icon.size(ICON_SIZE),
            Some(Message::SortDirectionToggled),
            Edge::Last,
        ),
        direction_tip,
    );

    let (view_icon, view_tip) = if app.settings.card_view {
        (lucide::table(), "Show as table")
    } else {
        (lucide::layout_grid(), "Show as cards")
    };
    let mut items: Vec<Element<'_, Message>> = vec![
        text("Sort by").size(13).color(MUTED).into(),
        input_group(vec![sort_key.into(), direction]),
        with_tooltip(
            icon_button(
                view_icon.size(ICON_SIZE),
                Some(Message::CardViewToggled),
                Edge::Alone,
            ),
            view_tip,
        ),
        with_tooltip(
            icon_button(
                lucide::chart_column().size(ICON_SIZE),
                has_hosts.then_some(Message::ShowSummary),
                Edge::Alone,
            ),
            "Summary",
        ),
    ];
    if wide {
        items.push(space::horizontal().into());
    }
    items.push(input_group(vec![
        history.into(),
        with_tooltip(
            icon_button(
                lucide::clipboard_copy().size(ICON_SIZE),
                has_hosts.then_some(Message::CopyVisibleRows),
                Edge::Middle,
            ),
            "Copy visible rows",
        ),
        export.into(),
    ]));
    toolbar_row(items, wide)
}

fn status_line(app: &LanPing) -> Element<'_, Message> {
    let (tint, message) = match &app.notice {
        Some((Notice::Info, message)) => (GOOD, message.as_str()),
        Some((Notice::Error, message)) => (BAD, message.as_str()),
        None => (MUTED, "Ready"),
    };
    row![
        text("*").color(tint).size(14),
        text(message).size(13).color(TEXT),
    ]
    .spacing(8)
    .into()
}

fn panel(content: Element<'_, Message>) -> Element<'_, Message> {
    container(content)
        .padding(PANEL_PADDING)
        .width(Length::Fill)
        .style(theme::panel)
        .into()
}
