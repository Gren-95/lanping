//! Search box, presets and the collapsible panel of filter dropdowns.

use iced::widget::{Row, button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Element, Length};
use lanping_core::device_type::DeviceType;

use crate::app::{LanPing, Message, SEARCH_INPUT_ID};
use crate::host_filter::{
    Choice, FilterChange, LatencyBand, NameFilter, RowStatus, port_choices, vendor_choices,
};
use crate::icons::{ICON_SIZE, icon_button, lucide, with_tooltip};
use crate::presets::{PresetChoice, all_presets};
use crate::theme;
use crate::theme::{Edge, ITEM_SPACING};
use crate::view::input_group;

const TEXT_SIZE: u32 = 13;
const ROW_SPACING: u32 = 8;

pub fn filter_bar(app: &LanPing, wide: bool) -> Element<'_, Message> {
    let mut bar = column![controls_row(app, wide)].spacing(ROW_SPACING);
    if app.filters_open {
        bar = bar.push(dropdowns_row(app));
    }
    bar.into()
}

fn controls_row(app: &LanPing, wide: bool) -> Element<'_, Message> {
    let active = app.filter.active_count();
    let search_edge = if wide { Edge::First } else { Edge::Alone };
    let search = text_input(
        "Search address, name, MAC, vendor or port",
        &app.filter.text,
    )
    .id(SEARCH_INPUT_ID)
    .on_input(|text| Message::Filter(FilterChange::Text(text)))
    .on_submit(Message::Noop)
    .padding(theme::CONTROL_PADDING)
    .size(TEXT_SIZE)
    .width(Length::Fill)
    .style(theme::input_at(search_edge));
    let presets_edge = if wide { Edge::Middle } else { Edge::First };
    let presets = pick_list(
        all_presets(&app.settings.filter_presets),
        None::<PresetChoice>,
        Message::PresetPicked,
    )
    .placeholder("Presets")
    .text_size(TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(if wide {
        Length::Fixed(190.0)
    } else {
        Length::Fill
    })
    .style(theme::pick_at(presets_edge));
    let toggle_icon: Element<'_, Message> = if active == 0 {
        lucide::funnel().size(ICON_SIZE).into()
    } else {
        row![
            lucide::funnel().size(ICON_SIZE),
            text(active.to_string()).size(TEXT_SIZE)
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    };
    let toggle = button(toggle_icon)
        .on_press(Message::FiltersPanelToggled)
        .padding([6.0, 10.0])
        .style(theme::button_at(
            Edge::Middle,
            theme::tab_button(app.filters_open),
        ));
    let clear = icon_button(
        lucide::funnel_x().size(ICON_SIZE),
        (active > 0).then_some(Message::Filter(FilterChange::Clear)),
        Edge::Middle,
    );
    let save = icon_button(
        lucide::bookmark_plus().size(ICON_SIZE),
        (active > 0).then_some(Message::ShowSavePreset),
        Edge::Last,
    );
    let actions = [
        presets.into(),
        with_tooltip(
            toggle,
            if app.filters_open {
                "Hide filters"
            } else {
                "Show filters"
            },
        ),
        with_tooltip(clear, "Clear filters"),
        with_tooltip(save, "Save filters as preset"),
    ];

    if wide {
        input_group(std::iter::once(search.into()).chain(actions).collect())
    } else {
        column![search, input_group(actions.into())]
            .spacing(ROW_SPACING)
            .into()
    }
}

fn dropdowns_row(app: &LanPing) -> Element<'_, Message> {
    let filter = &app.filter;
    let statuses: Vec<Choice<RowStatus>> = std::iter::once(Choice::Any)
        .chain(RowStatus::FILTERABLE.into_iter().map(Choice::Only))
        .collect();

    let device_types: Vec<Choice<DeviceType>> = std::iter::once(Choice::Any)
        .chain(DeviceType::ALL.into_iter().map(Choice::Only))
        .collect();

    let dropdowns = Row::with_children(vec![
        dropdown(device_types, filter.device_type.into(), 170.0, |choice| {
            Message::Filter(FilterChange::DeviceType(choice))
        }),
        dropdown(LatencyBand::ALL.to_vec(), filter.latency, 190.0, |band| {
            Message::Filter(FilterChange::Latency(band))
        }),
        dropdown(statuses, filter.status.into(), 140.0, |choice| {
            Message::Filter(FilterChange::Status(choice))
        }),
        dropdown(
            vendor_choices(&app.hosts),
            filter.vendor.clone().into(),
            200.0,
            |choice| Message::Filter(FilterChange::Vendor(choice)),
        ),
        dropdown(
            port_choices(&app.hosts),
            filter.port.into(),
            160.0,
            |choice| Message::Filter(FilterChange::Port(choice)),
        ),
        dropdown(NameFilter::ALL.to_vec(), filter.name, 160.0, |name| {
            Message::Filter(FilterChange::Name(name))
        }),
    ])
    .spacing(ITEM_SPACING)
    .align_y(Alignment::Center)
    .wrap()
    .vertical_spacing(ROW_SPACING);

    container(dropdowns).style(theme::panel).padding(10).into()
}

fn dropdown<'a, T>(
    options: Vec<T>,
    selected: T,
    width: f32,
    on_pick: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message>
where
    T: ToString + PartialEq + Clone + 'a,
{
    pick_list(options, Some(selected), on_pick)
        .text_size(TEXT_SIZE)
        .padding(theme::CONTROL_PADDING)
        .width(width)
        .style(theme::pick)
        .into()
}
