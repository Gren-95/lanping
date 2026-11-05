//! The Web tab pages: how a website answers and what a domain's email records say.

use iced::widget::{button, column, row, text, text_input};
use iced::{Alignment, Element, Length};
use lanping_core::email_check::{EmailReport, RecordCheck};
use lanping_core::verdict::Verdict;
use lanping_core::website_check::{HeaderCheck, Timings, WebsiteReport};

use crate::app::{LanPing, Message};
use crate::icons::{ICON_SIZE, lucide, with_tooltip};
use crate::results::pill;
use crate::theme::{self, BAD, BOLD, Edge, GOOD, MUTED, SECTION_SPACING, WARN};
use crate::ui::{card, empty_state, fact_row, note, page_header, scrolling};
use crate::view::input_group;
use crate::web_tab::{Lookup, WebState};

const INPUT_TEXT_SIZE: u32 = 13;
const SECTION_TITLE_SIZE: u32 = 15;
const HEADLINE_SIZE: u32 = 20;
const MILLISECONDS: f64 = 1000.0;

/// The Website page of the DNS tab.
pub fn website_page(app: &LanPing) -> Element<'_, Message> {
    let web = &app.web;
    column![
        page_header(
            "Website check",
            "How a site answers: redirects, where the time goes, the TLS version and its security headers.",
            None,
        ),
        controls(
            "Web address, such as example.com or https://example.com/page",
            &web.address,
            web.site.is_running(),
            Message::WebAddressChanged,
            Message::WebCheck,
        ),
        site_content(web),
    ]
    .spacing(SECTION_SPACING)
    .height(Length::Fill)
    .into()
}

/// The Email records page of the DNS tab.
pub fn email_page(app: &LanPing) -> Element<'_, Message> {
    let web = &app.web;
    column![
        page_header(
            "Email records",
            "Where a domain's mail goes and whether SPF, DMARC and DKIM protect it from forgery.",
            None,
        ),
        controls(
            "Domain, such as example.com",
            &web.domain,
            web.email.is_running(),
            Message::EmailDomainChanged,
            Message::EmailCheck,
        ),
        email_content(web),
    ]
    .spacing(SECTION_SPACING)
    .height(Length::Fill)
    .into()
}

fn controls<'a>(
    placeholder: &'static str,
    value: &'a str,
    busy: bool,
    on_input: fn(String) -> Message,
    on_submit: Message,
) -> Element<'a, Message> {
    let mut field = text_input(placeholder, value)
        .padding(theme::CONTROL_PADDING)
        .size(INPUT_TEXT_SIZE)
        .width(Length::Fill)
        .style(theme::input_at(Edge::First));
    if !busy {
        field = field.on_input(on_input).on_submit(on_submit.clone());
    }
    let go = with_tooltip(
        button(lucide::globe().size(ICON_SIZE + 2.0))
            .on_press_maybe((!busy).then_some(on_submit))
            .padding([7.0, 20.0])
            .style(theme::button_at(Edge::Last, theme::primary_button)),
        "Check",
    );
    input_group(vec![field.into(), go])
}

fn site_content(web: &WebState) -> Element<'_, Message> {
    match &web.site {
        Lookup::Idle => empty_state(
            "Not checked yet",
            "Enter an address and press the button. It makes ordinary requests, like a browser.",
        ),
        Lookup::Running => empty_state("Checking", web.address.trim().to_owned()),
        Lookup::Failed(error) => empty_state("Cannot check the site", error.to_string()),
        Lookup::Done(report) => scrolling(site_report(report)).into(),
    }
}

fn email_content(web: &WebState) -> Element<'_, Message> {
    match &web.email {
        Lookup::Idle => empty_state(
            "Not checked yet",
            "Enter a domain and press the button. It asks ordinary DNS questions.",
        ),
        Lookup::Running => empty_state("Checking", web.domain.trim().to_owned()),
        Lookup::Failed(error) => empty_state("Cannot check the domain", error.to_string()),
        Lookup::Done(report) => scrolling(email_report(report)).into(),
    }
}

fn site_report(report: &WebsiteReport) -> Element<'_, Message> {
    let last = report.hops.last();
    let verdict = match last.map_or(0, |hop| hop.status) {
        200..=299 => pill("Answers", GOOD),
        300..=399 => pill("Redirects", WARN),
        _ => pill("Error answer", BAD),
    };
    let headline = row![
        text(last.map_or_else(String::new, |hop| format!("{}  {}", hop.status, hop.url)))
            .size(HEADLINE_SIZE)
            .font(BOLD),
        verdict
    ]
    .spacing(12)
    .align_y(Alignment::Center);
    let mut summary = column![headline].spacing(6);
    if let Some(version) = &report.tls_version {
        summary = summary.push(fact_row("Secure connection", version.clone()));
    }
    if let Some(reason) = &report.stopped {
        summary = summary.push(text(reason.as_str()).color(WARN));
    }
    column![
        card(summary),
        card(redirect_chain(report)),
        card(timing_rows(&report.timings)),
        card(header_rows(&report.headers)),
    ]
    .spacing(SECTION_SPACING)
    .into()
}

fn section_title(title: &'static str) -> Element<'static, Message> {
    text(title).size(SECTION_TITLE_SIZE).font(BOLD).into()
}

fn redirect_chain(report: &WebsiteReport) -> Element<'_, Message> {
    let mut chain = column![section_title("Requests")].spacing(6);
    if report.hops.len() == 1 {
        chain = chain.push(note("No redirects: the first address answered."));
    }
    for hop in &report.hops {
        let target = hop
            .location
            .as_deref()
            .filter(|_| (300..400).contains(&hop.status))
            .map_or_else(String::new, |location| format!("  →  {location}"));
        chain = chain.push(fact_row_plain(
            format!("{}", hop.status),
            format!("{}{target}", hop.url),
            format_milliseconds(hop.total.as_secs_f64()),
        ));
    }
    chain.into()
}

/// A line of the request chain: a status, an address and how long it took.
fn fact_row_plain<'a>(status: String, address: String, time: String) -> Element<'a, Message> {
    row![
        text(status).color(MUTED).width(50.0),
        text(address).width(Length::Fill),
        text(time).size(12).color(MUTED),
    ]
    .spacing(8)
    .align_y(Alignment::Start)
    .into()
}

fn timing_rows(timings: &Timings) -> Element<'_, Message> {
    let mut rows = column![section_title("Where the time went")].spacing(6);
    let mut steps = vec![
        ("Name lookup", Some(timings.dns)),
        ("Connect", Some(timings.connect)),
        ("Secure handshake", timings.tls),
        ("Wait for first byte", Some(timings.first_byte)),
    ];
    steps.retain(|(_, time)| time.is_some());
    for (label, time) in steps {
        rows = rows.push(fact_row(
            label,
            format_milliseconds(time.map_or(0.0, |time| time.as_secs_f64())),
        ));
    }
    rows.push(fact_row(
        "Total",
        format_milliseconds(timings.total.as_secs_f64()),
    ))
    .into()
}

fn format_milliseconds(seconds: f64) -> String {
    format!("{:.0} ms", seconds * MILLISECONDS)
}

fn header_rows(headers: &[HeaderCheck]) -> Element<'_, Message> {
    headers
        .iter()
        .fold(
            column![section_title("Security headers")].spacing(10),
            |list, check| list.push(header_row(check)),
        )
        .into()
}

fn header_row(check: &HeaderCheck) -> Element<'_, Message> {
    let verdict = match (check.verdict, check.value.is_some()) {
        (Verdict::Good, _) => pill("Good", GOOD),
        (_, false) => pill("Missing", WARN),
        (_, true) => pill("Weak", WARN),
    };
    let mut body = column![
        row![text(check.name).font(BOLD), verdict]
            .spacing(10)
            .align_y(Alignment::Center),
        text(check.advice.as_str()).size(13).color(MUTED),
    ]
    .spacing(3);
    if let Some(value) = &check.value {
        body = body.push(text(value.as_str()).size(12).font(iced::Font::MONOSPACE));
    }
    body.into()
}

fn email_report(report: &EmailReport) -> Element<'_, Message> {
    column![
        card(mail_servers(report)),
        card(record_check(
            "SPF",
            "Who may send mail for the domain",
            &report.spf
        )),
        card(record_check(
            "DMARC",
            "What to do with mail that fails the checks",
            &report.dmarc
        )),
        card(dkim_rows(report)),
    ]
    .spacing(SECTION_SPACING)
    .into()
}

fn mail_servers(report: &EmailReport) -> Element<'_, Message> {
    let mut body = column![section_title("Where mail goes (MX)")].spacing(6);
    if report.mail_servers.is_empty() {
        return body
            .push(note(
                "No MX records. Mail for this domain has nowhere to go, unless the sender \
                 falls back to the domain's own address.",
            ))
            .into();
    }
    for server in &report.mail_servers {
        body = body.push(fact_row_plain(
            server.preference.to_string(),
            server.host.clone(),
            String::new(),
        ));
    }
    body.push(note("The lowest number is tried first.")).into()
}

fn verdict_pill(check: &RecordCheck) -> Element<'static, Message> {
    match (check.verdict, check.record.is_some()) {
        (Verdict::Good, _) => pill("Good", GOOD),
        (Verdict::Weak, _) => pill("Could be stronger", WARN),
        (Verdict::Missing, false) => pill("Missing", BAD),
        (Verdict::Missing, true) => pill("Problem", BAD),
    }
}

fn record_check<'a>(
    title: &'static str,
    purpose: &'static str,
    check: &'a RecordCheck,
) -> Element<'a, Message> {
    let mut body = column![
        row![
            text(title).size(SECTION_TITLE_SIZE).font(BOLD),
            verdict_pill(check),
            text(purpose).size(13).color(MUTED)
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        text(check.meaning.as_str()),
    ]
    .spacing(6);
    if let Some(record) = &check.record {
        body = body.push(text(record.as_str()).size(12).font(iced::Font::MONOSPACE));
    }
    body.into()
}

fn dkim_rows(report: &EmailReport) -> Element<'_, Message> {
    let mut body = column![
        row![
            text("DKIM").size(SECTION_TITLE_SIZE).font(BOLD),
            text("Signing keys at common names").size(13).color(MUTED)
        ]
        .spacing(10)
    ]
    .spacing(6);
    if report.dkim_selectors.is_empty() {
        return body
            .push(note(
                "No key found at the common names. That does not mean there is none: a key \
                 under another name cannot be found this way.",
            ))
            .into();
    }
    body = body.push(pill("Found", GOOD));
    for selector in &report.dkim_selectors {
        body = body.push(fact_row("Selector", selector.clone()));
    }
    body.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_shown_in_whole_milliseconds() {
        assert_eq!(format_milliseconds(0.0584), "58 ms");
        assert_eq!(format_milliseconds(1.5), "1500 ms");
    }
}
