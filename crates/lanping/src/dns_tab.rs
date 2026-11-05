//! The DNS tab: look up records at a chosen resolver and compare resolver speed.

use std::fmt;
use std::net::IpAddr;

use iced::widget::{
    Row as LayoutRow, button, column, container, pick_list, progress_bar, row, scrollable, space,
    text, text_input,
};
use iced::{Alignment, Element, Length, Task};
use lanping_core::dns::{
    Answer, BENCHMARK_ROUNDS, DnsError, PUBLIC_RESOLVERS, RecordType, ResolverTiming, benchmark,
    lookup, system_resolvers,
};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, copy_button, icon_button, lucide, with_tooltip};
use crate::job::Job;
use crate::mtu_tab::mtu_page;
use crate::results::pill;
use crate::table_grid::{cell_width, grid_line};
use crate::theme::{
    self, BAD, BOLD, Edge, GOOD, MUTED, NO_VALUE, SECTION_SPACING, WARN, latency_color,
};
use crate::ui::{note, page_strip, panel, scrolling};
use crate::view::input_group;
use crate::web_view::{email_page, website_page};
use crate::whois_page::{WhoisState, whois_view};

/// Windows at least this wide keep the lookup controls on one line.
const WIDE_WIDTH: f32 = 820.0;
const CONTROL_TEXT_SIZE: u32 = 13;
const TYPE_PICK_WIDTH: f32 = 100.0;
const RESOLVER_PICK_WIDTH: f32 = 240.0;
const BAR_LENGTH: f32 = 160.0;
const BAR_GIRTH: f32 = 8.0;

/// A resolver to ask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolverChoice {
    label: String,
    address: IpAddr,
}

impl fmt::Display for ResolverChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.label, self.address)
    }
}

/// The resolvers the system uses first, then the public ones.
fn resolver_choices() -> Vec<ResolverChoice> {
    let system = system_resolvers()
        .into_iter()
        .map(|address| ResolverChoice {
            label: "System".to_owned(),
            address,
        });
    let public = PUBLIC_RESOLVERS
        .iter()
        .map(|(label, address)| ResolverChoice {
            label: (*label).to_owned(),
            address: *address,
        });
    system.chain(public).collect()
}

/// The pages of the tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DnsPage {
    #[default]
    Lookup,
    Whois,
    Website,
    Email,
    Mtu,
}

impl DnsPage {
    pub(crate) const ALL: [Self; 5] = [
        Self::Lookup,
        Self::Whois,
        Self::Website,
        Self::Email,
        Self::Mtu,
    ];

    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::Lookup => "Lookup",
            Self::Whois => "WHOIS",
            Self::Website => "Website",
            Self::Email => "Email records",
            Self::Mtu => "MTU",
        }
    }
}

enum Lookup {
    Idle,
    Running,
    Done(Answer),
    Failed(DnsError),
}

enum Comparison {
    Idle,
    Running,
    Done(Vec<ResolverTiming>),
}

/// What is typed and chosen, and what the last lookup and comparison found.
pub struct DnsState {
    page: DnsPage,
    pub whois: WhoisState,
    name: String,
    record_type: RecordType,
    choices: Vec<ResolverChoice>,
    resolver: ResolverChoice,
    lookup: Lookup,
    comparison: Comparison,
    /// The lookup and the comparison are separate runs, so one does not drop the other's answer.
    lookup_job: Job,
    comparison_job: Job,
}

impl DnsState {
    pub fn new() -> Self {
        let choices = resolver_choices();
        // The public resolvers are always listed, so there is always a first choice.
        let resolver = choices[0].clone();
        Self {
            page: DnsPage::default(),
            whois: WhoisState::new(),
            name: String::new(),
            record_type: RecordType::A,
            choices,
            resolver,
            lookup: Lookup::Idle,
            comparison: Comparison::Idle,
            lookup_job: Job::default(),
            comparison_job: Job::default(),
        }
    }

    pub fn set_page(&mut self, page: DnsPage) {
        self.page = page;
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    pub fn set_record_type(&mut self, record_type: RecordType) {
        self.record_type = record_type;
    }

    pub fn set_resolver(&mut self, resolver: ResolverChoice) {
        self.resolver = resolver;
    }

    pub fn start_lookup(&mut self) -> Task<Message> {
        self.lookup = Lookup::Running;
        let (server, name, record_type) =
            (self.resolver.address, self.name.clone(), self.record_type);
        self.lookup_job.spawn(
            async move { lookup(server, &name, record_type).await },
            Message::DnsLookupDone,
        )
    }

    pub fn finish_lookup(&mut self, generation: u64, result: Result<Answer, DnsError>) {
        if !self.lookup_job.is_current(generation) {
            return;
        }
        self.lookup_job.finished();
        self.lookup = match result {
            Ok(answer) => Lookup::Done(answer),
            Err(error) => Lookup::Failed(error),
        };
    }

    pub fn start_comparison(&mut self) -> Task<Message> {
        self.comparison = Comparison::Running;
        let resolvers: Vec<(String, IpAddr)> = self
            .choices
            .iter()
            .map(|choice| (choice.label.clone(), choice.address))
            .collect();
        let name = self.name.clone();
        self.comparison_job.spawn(
            async move { benchmark(resolvers, &name).await },
            Message::DnsComparisonDone,
        )
    }

    pub fn finish_comparison(&mut self, generation: u64, timings: Vec<ResolverTiming>) {
        if self.comparison_job.is_current(generation) {
            self.comparison_job.finished();
            self.comparison = Comparison::Done(timings);
        }
    }

    /// The records of the last answer as tab-separated text, and how many there are.
    pub fn records_as_text(&self) -> (String, usize) {
        let Lookup::Done(answer) = &self.lookup else {
            return (String::new(), 0);
        };
        let lines = answer
            .records
            .iter()
            .map(|record| format!("{}\t{}\t{}", record.type_name(), record.ttl, record.data));
        (lines.collect::<Vec<_>>().join("\n"), answer.records.len())
    }
}

pub fn dns_panel(app: &LanPing, width: f32) -> Element<'_, Message> {
    let dns = &app.dns;
    let strip = page_strip(
        &DnsPage::ALL,
        dns.page,
        DnsPage::title,
        Message::DnsPagePicked,
    );
    let body = match dns.page {
        DnsPage::Lookup => column![
            lookup_controls(dns, width >= WIDE_WIDTH),
            lookup_result(dns),
            comparison_section(dns),
        ]
        .spacing(SECTION_SPACING * 1.5),
        DnsPage::Whois => column![whois_view(&dns.whois)],
        // These pages have empty states that fill the page, so they do not scroll as a whole.
        DnsPage::Website | DnsPage::Email | DnsPage::Mtu => {
            let page = match dns.page {
                DnsPage::Website => website_page(app),
                DnsPage::Email => email_page(app),
                _ => mtu_page(app),
            };
            return panel(
                column![strip, page]
                    .spacing(SECTION_SPACING)
                    .height(Length::Fill),
            );
        }
    };
    panel(scrolling(column![strip, body].spacing(SECTION_SPACING)))
}

fn lookup_controls(dns: &DnsState, wide: bool) -> Element<'_, Message> {
    let busy = matches!(dns.lookup, Lookup::Running);
    let name_edge = if wide { Edge::First } else { Edge::Alone };
    let mut name = text_input("Host name, or an address for a PTR lookup", &dns.name)
        .padding(theme::CONTROL_PADDING)
        .size(CONTROL_TEXT_SIZE)
        .width(Length::Fill)
        .style(theme::input_at(name_edge));
    if !busy {
        name = name
            .on_input(Message::DnsNameChanged)
            .on_submit(Message::DnsLookup);
    }
    let type_edge = if wide { Edge::Middle } else { Edge::First };
    let record_type = pick_list(
        RecordType::ALL,
        Some(dns.record_type),
        Message::DnsTypePicked,
    )
    .text_size(CONTROL_TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(TYPE_PICK_WIDTH)
    .style(theme::pick_at(type_edge));
    let resolver = pick_list(
        dns.choices.as_slice(),
        Some(&dns.resolver),
        Message::DnsResolverPicked,
    )
    .text_size(CONTROL_TEXT_SIZE)
    .padding(theme::CONTROL_PADDING)
    .width(RESOLVER_PICK_WIDTH)
    .style(theme::pick_at(Edge::Middle));
    let go = with_tooltip(
        button(lucide::search().size(ICON_SIZE + 2.0))
            .on_press_maybe((!busy).then_some(Message::DnsLookup))
            .padding([7.0, 20.0])
            .style(theme::button_at(Edge::Last, theme::primary_button)),
        "Look up",
    );
    let rest = [record_type.into(), resolver.into(), go];
    if wide {
        input_group(std::iter::once(name.into()).chain(rest).collect())
    } else {
        column![name, input_group(rest.into())].spacing(8).into()
    }
}

fn lookup_result(dns: &DnsState) -> Element<'_, Message> {
    match &dns.lookup {
        Lookup::Idle => note("Enter a name and press the search button."),
        Lookup::Running => note(format!("Asking {}.", dns.resolver)),
        Lookup::Failed(error) => container(text(error.to_string()).color(BAD)).into(),
        Lookup::Done(answer) => answer_table(dns, answer),
    }
}

const RECORD_COLUMNS: [(&str, f32); 3] = [("Type", 70.0), ("TTL", 80.0), ("Value", 520.0)];

fn answer_table<'a>(dns: &'a DnsState, answer: &'a Answer) -> Element<'a, Message> {
    let milliseconds = answer.elapsed.as_secs_f64() * 1000.0;
    let copy_all = with_tooltip(
        icon_button(
            lucide::clipboard_copy().size(ICON_SIZE),
            (!answer.records.is_empty()).then_some(Message::DnsCopyRecords),
            Edge::Alone,
        ),
        "Copy records",
    );
    let summary = LayoutRow::with_children([
        pill(format!("{} records", answer.records.len()), theme::ACCENT),
        pill(format!("{milliseconds:.1} ms"), latency_color(milliseconds)),
        text(format!("from {}", answer.server))
            .size(12)
            .color(MUTED)
            .into(),
        space::horizontal().into(),
        copy_all,
    ])
    .spacing(8)
    .align_y(Alignment::Center);
    if answer.records.is_empty() {
        return column![
            summary,
            note(format!(
                "No {} records for {}.",
                dns.record_type,
                dns.name.trim()
            ))
        ]
        .spacing(8)
        .into();
    }
    let header = grid_line(
        RECORD_COLUMNS
            .iter()
            .map(|(title, width)| {
                (
                    cell_width(*width),
                    text(*title).size(13).color(MUTED).into(),
                )
            })
            .collect(),
    );
    let lines = answer
        .records
        .iter()
        .enumerate()
        .fold(column![header], |list, (index, record)| {
            let value = row![
                text(record.data.as_str()),
                copy_button(record.data.clone(), "value")
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            let cells = vec![
                (
                    cell_width(RECORD_COLUMNS[0].1),
                    text(record.type_name()).font(BOLD).into(),
                ),
                (
                    cell_width(RECORD_COLUMNS[1].1),
                    text(record.ttl.to_string()).into(),
                ),
                (cell_width(RECORD_COLUMNS[2].1), value.into()),
            ];
            list.push(container(grid_line(cells)).style(theme::stripe(index % 2 == 1)))
        });
    column![
        summary,
        scrollable(lines).direction(scrollable::Direction::Horizontal(
            scrollable::Scrollbar::default()
        ))
    ]
    .spacing(8)
    .into()
}

fn comparison_section(dns: &DnsState) -> Element<'_, Message> {
    let running = matches!(dns.comparison, Comparison::Running);
    let run = with_tooltip(
        button(lucide::gauge().size(ICON_SIZE + 2.0))
            .on_press_maybe((!running).then_some(Message::DnsCompare))
            .padding([7.0, 20.0])
            .style(theme::button_at(Edge::Alone, theme::primary_button)),
        "Compare resolvers",
    );
    let heading = row![
        text("Compare resolvers").size(16).font(BOLD),
        space::horizontal(),
        run
    ]
    .align_y(Alignment::Center);
    let hint = note(format!(
        "Times {BENCHMARK_ROUNDS} lookups of the name above at every resolver in the list. \
         Use a name that exists; the first answer may be slower while the resolver looks it up."
    ));
    let result: Element<'_, Message> = match &dns.comparison {
        Comparison::Idle => space::vertical().height(0).into(),
        Comparison::Running => note("Asking every resolver."),
        Comparison::Done(timings) => timing_table(timings),
    };
    column![heading, hint, result].spacing(8).into()
}

fn timing_table(timings: &[ResolverTiming]) -> Element<'_, Message> {
    let slowest = timings
        .iter()
        .filter_map(ResolverTiming::average_ms)
        .fold(1.0, f64::max);
    timings
        .iter()
        .fold(column![].spacing(6), |list, timing| {
            let result: Element<'_, Message> = match timing.average_ms() {
                Some(average) => row![
                    progress_bar(0.0..=slowest as f32, average as f32)
                        .girth(BAR_GIRTH)
                        .length(BAR_LENGTH)
                        .style(theme::progress),
                    text(format!("{average:.1} ms")).color(latency_color(average)),
                    text(format!("{}/{BENCHMARK_ROUNDS} answered", timing.answered()))
                        .size(12)
                        .color(if timing.answered() == BENCHMARK_ROUNDS {
                            GOOD
                        } else {
                            WARN
                        }),
                ]
                .spacing(10)
                .align_y(Alignment::Center)
                .into(),
                None => text(
                    timing
                        .last_error()
                        .map_or_else(|| NO_VALUE.to_owned(), ToString::to_string),
                )
                .size(13)
                .color(BAD)
                .into(),
            };
            list.push(
                row![
                    text(format!("{} ({})", timing.name, timing.address))
                        .width(RESOLVER_PICK_WIDTH),
                    result
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_public_resolvers_are_always_offered() {
        let choices = resolver_choices();
        assert!(choices.len() >= PUBLIC_RESOLVERS.len());
        assert!(choices.iter().any(|choice| choice.label == "Cloudflare"));
    }

    #[test]
    fn a_choice_shows_its_label_and_address() {
        let choice = ResolverChoice {
            label: "Quad9".to_owned(),
            address: "9.9.9.9".parse().unwrap(),
        };
        assert_eq!(choice.to_string(), "Quad9 (9.9.9.9)");
    }

    #[test]
    fn an_overtaken_answer_is_ignored() {
        let mut state = DnsState::new();
        let _ = state.start_lookup();
        let _ = state.start_lookup();
        state.finish_lookup(1, Err(DnsError::Timeout));
        assert!(matches!(state.lookup, Lookup::Running));
        state.finish_lookup(2, Err(DnsError::Timeout));
        assert!(matches!(state.lookup, Lookup::Failed(DnsError::Timeout)));
    }

    #[test]
    fn a_comparison_started_during_a_lookup_does_not_drop_the_lookup_answer() {
        let mut state = DnsState::new();
        let _ = state.start_lookup();
        let _ = state.start_comparison();
        state.finish_lookup(1, Err(DnsError::Timeout));
        assert!(matches!(state.lookup, Lookup::Failed(DnsError::Timeout)));
        state.finish_comparison(1, Vec::new());
        assert!(matches!(state.comparison, Comparison::Done(_)));
    }

    #[test]
    fn records_have_no_text_before_an_answer() {
        assert_eq!(DnsState::new().records_as_text(), (String::new(), 0));
    }
}
