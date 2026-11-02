//! A self-contained HTML inventory report. It prints cleanly, so a browser can save it as a PDF.

use std::fmt::Write;

use crate::device_notes::DeviceNotes;
use crate::device_type::guess_device_type;
use crate::host::Host;
use crate::labels::DeviceLabels;
use crate::ports::format_ports;
use crate::summary::summarize;

const STYLE: &str = "body{font:14px/1.45 system-ui,sans-serif;color:#1b2230;margin:32px}\
h1{margin:0 0 4px}h2{margin:28px 0 8px;font-size:16px}.meta{color:#5b6677}\
table{border-collapse:collapse;width:100%}th,td{text-align:left;padding:6px 10px;\
border-bottom:1px solid #d9dee6;vertical-align:top}th{font-size:12px;color:#5b6677}\
tbody tr:nth-child(even){background:#f4f6f9}.totals{display:flex;flex-wrap:wrap;gap:32px}\
.totals table{width:auto}code{font-size:12px}@media print{body{margin:0}tr{break-inside:avoid}}";
const NO_VALUE: &str = "-";
const TOTAL_ROWS: usize = 8;

/// What the report is about and the facts the user added.
pub struct ReportInput<'a> {
    pub target: &'a str,
    pub generated_at: &'a str,
    pub hosts: &'a [Host],
    pub labels: &'a DeviceLabels,
    pub notes: &'a DeviceNotes,
}

/// Device text comes from the network, so everything is escaped before it goes in the page.
fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

fn cell_row(cells: &[String]) -> String {
    let cells: String = cells
        .iter()
        .map(|cell| format!("<td>{}</td>", escape(cell)))
        .collect();
    format!("<tr>{cells}</tr>\n")
}

fn totals_table(title: &str, rows: Vec<(String, usize)>) -> String {
    let body: String = rows
        .into_iter()
        .take(TOTAL_ROWS)
        .map(|(name, count)| cell_row(&[name, count.to_string()]))
        .collect();
    format!(
        "<div><h2>{}</h2><table><tbody>{body}</tbody></table></div>",
        escape(title)
    )
}

fn device_row(host: &Host, input: &ReportInput<'_>) -> String {
    let key = host.device_key();
    let name = input
        .labels
        .get(&key)
        .or(host.display_name())
        .unwrap_or(NO_VALUE);
    let note = input.notes.get(&key).cloned().unwrap_or_default();
    let or_none = |text: Option<&str>| text.unwrap_or(NO_VALUE).to_owned();
    cell_row(&[
        host.ip.to_string(),
        name.to_owned(),
        guess_device_type(host).to_string(),
        or_none(host.mac.as_deref()),
        or_none(host.vendor.as_deref()),
        format_ports(&host.open_ports),
        note.owner,
        note.location,
        note.tags.join(", "),
        note.notes,
    ])
}

/// Renders the report. Its totals describe only the devices in this list.
pub fn to_html(input: &ReportInput<'_>) -> String {
    let summary = summarize(input.hosts, &[], 0);
    let mut page = String::new();
    let title = format!("LanPing report: {}", input.target);
    let _ = write!(
        page,
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>{}</title>\
         <style>{STYLE}</style></head><body>\n<h1>{}</h1>\n\
         <p class=\"meta\">{} devices. Generated {}. Device types are best guesses.</p>\n\
         <div class=\"totals\">",
        escape(&title),
        escape(&title),
        summary.device_count,
        escape(input.generated_at),
    );
    let by_type = summary
        .by_type
        .iter()
        .map(|(kind, count)| (kind.to_string(), *count))
        .collect();
    let by_port = summary
        .by_port
        .iter()
        .map(|(port, count)| (format!("Port {port}"), *count))
        .collect();
    page.push_str(&totals_table("By type", by_type));
    page.push_str(&totals_table("By vendor", summary.by_vendor.clone()));
    page.push_str(&totals_table("Open ports", by_port));
    page.push_str("</div>\n<h2>Devices</h2>\n<table><thead><tr>");
    for heading in [
        "Address",
        "Name",
        "Type",
        "MAC",
        "Vendor",
        "Open ports",
        "Owner",
        "Location",
        "Tags",
        "Notes",
    ] {
        let _ = write!(page, "<th>{heading}</th>");
    }
    page.push_str("</tr></thead><tbody>\n");
    let mut hosts: Vec<&Host> = input.hosts.iter().collect();
    hosts.sort_by_key(|host| host.ip);
    for host in hosts {
        page.push_str(&device_row(host, input));
    }
    page.push_str("</tbody></table>\n</body></html>\n");
    page
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device_notes::DeviceNote;

    fn render(hosts: &[Host], labels: &DeviceLabels, notes: &DeviceNotes) -> String {
        to_html(&ReportInput {
            target: "10.0.0.0/24",
            generated_at: "2026-11-02 10:00 UTC",
            hosts,
            labels,
            notes,
        })
    }

    #[test]
    fn includes_totals_devices_and_the_users_facts() {
        let mut host = Host::new("10.0.0.5".parse().unwrap());
        host.open_ports = vec![22, 9100];
        let key = host.device_key();
        let mut labels = DeviceLabels::default();
        labels.set(&key, "Office printer");
        let mut notes = DeviceNotes::default();
        notes.set(&key, DeviceNote::from_input("Sam", "Attic", "lab", "old"));
        let html = render(&[host], &labels, &notes);
        for expected in [
            "10.0.0.0/24",
            "2026-11-02",
            "10.0.0.5",
            "Office printer",
            "22, 9100",
            "Sam",
            "Attic",
            "Printer",
            "Port 9100",
        ] {
            assert!(html.contains(expected), "{expected}");
        }
    }

    #[test]
    fn escapes_text_from_the_network() {
        let mut host = Host::new("10.0.0.5".parse().unwrap());
        host.hostname = Some("<script>alert('x')</script>".into());
        host.vendor = Some("A&B \"Co\"".into());
        let html = render(&[host], &DeviceLabels::default(), &DeviceNotes::default());
        assert!(!html.contains("<script"));
        assert!(html.contains("&lt;script&gt;alert(&#39;x&#39;)"));
        assert!(html.contains("A&amp;B &quot;Co&quot;"));
    }

    #[test]
    fn lists_devices_in_address_order() {
        let hosts = [
            Host::new("10.0.0.20".parse().unwrap()),
            Host::new("10.0.0.3".parse().unwrap()),
        ];
        let html = render(&hosts, &DeviceLabels::default(), &DeviceNotes::default());
        assert!(html.find("10.0.0.3<").unwrap() < html.find("10.0.0.20<").unwrap());
    }
}
