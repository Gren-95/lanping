use std::borrow::Cow;

use serde::Serialize;

use crate::device_notes::{DeviceNote, DeviceNotes};
use crate::host::Host;
use crate::rtt_stats::millis;

/// The columns of the CSV export, in order. [`host_fields`] fills them in the same order.
const CSV_COLUMNS: [&str; 16] = [
    "ip",
    "latency_ms",
    "hostname",
    "mac",
    "vendor",
    "open_ports",
    "banners",
    "ipv6",
    "device_name",
    "manufacturer",
    "model",
    "services",
    "owner",
    "location",
    "tags",
    "notes",
];

pub fn to_csv(hosts: &[Host], notes: &DeviceNotes) -> String {
    let mut output = CSV_COLUMNS.join(",");
    output.push('\n');
    let no_note = DeviceNote::default();
    for host in hosts {
        let note = notes.get(&host.device_key()).unwrap_or(&no_note);
        push_row(&mut output, &host_fields(host, note));
    }
    output
}

/// The values of one host, one per entry of [`CSV_COLUMNS`]. Text that already exists is
/// borrowed; only values that have to be built are allocated.
fn host_fields<'a>(host: &'a Host, note: &'a DeviceNote) -> [Cow<'a, str>; CSV_COLUMNS.len()] {
    let borrowed = |text: &'a Option<String>| Cow::Borrowed(text.as_deref().unwrap_or_default());
    let joined = |parts: Vec<String>, separator: &str| Cow::Owned(parts.join(separator));
    [
        Cow::Owned(host.ip.to_string()),
        Cow::Owned(
            host.latency
                .map(|latency| format!("{:.1}", millis(latency)))
                .unwrap_or_default(),
        ),
        borrowed(&host.hostname),
        borrowed(&host.mac),
        borrowed(&host.vendor),
        joined(host.open_ports.iter().map(u16::to_string).collect(), " "),
        joined(
            host.banners
                .iter()
                .map(|banner| format!("{}: {}", banner.port, banner.text))
                .collect(),
            "; ",
        ),
        joined(host.ipv6.iter().map(ToString::to_string).collect(), " "),
        borrowed(&host.discovery.name),
        borrowed(&host.discovery.manufacturer),
        borrowed(&host.discovery.model),
        Cow::Owned(host.discovery.services.join(" ")),
        Cow::Borrowed(&note.owner),
        Cow::Borrowed(&note.location),
        Cow::Owned(note.tags.join(" ")),
        Cow::Borrowed(&note.notes),
    ]
}

/// Appends one CSV line.
fn push_row(output: &mut String, fields: &[Cow<'_, str>]) {
    for (index, field) in fields.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&escape_csv_field(field));
    }
    output.push('\n');
}

/// One host in the JSON export, with the user's note beside it when there is one.
#[derive(Serialize)]
struct ExportedHost<'a> {
    #[serde(flatten)]
    host: &'a Host,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'a DeviceNote>,
}

pub fn to_json(hosts: &[Host], notes: &DeviceNotes) -> serde_json::Result<String> {
    let rows: Vec<ExportedHost<'_>> = hosts
        .iter()
        .map(|host| ExportedHost {
            host,
            note: notes.get(&host.device_key()),
        })
        .collect();
    serde_json::to_string_pretty(&rows)
}

/// Characters that make a spreadsheet read a value as a formula. A tab or a carriage return
/// in front of one can also start it.
const FORMULA_STARTS: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// Quotes fields that need it (RFC 4180) and defuses spreadsheet formulas by prefixing a
/// quote to values that start with a formula character.
fn escape_csv_field(field: &str) -> Cow<'_, str> {
    let defused = if field.starts_with(FORMULA_STARTS) {
        Cow::Owned(format!("'{field}"))
    } else {
        Cow::Borrowed(field)
    };
    if defused.contains([',', '"', '\n', '\r']) {
        Cow::Owned(format!("\"{}\"", defused.replace('"', "\"\"")))
    } else {
        defused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn sample_host() -> Host {
        let mut host = Host::new("192.168.0.2".parse().unwrap());
        host.latency = Some(Duration::from_micros(1500));
        host.hostname = Some("nas, main".into());
        host.mac = Some("AA:BB:CC:DD:EE:FF".into());
        host.open_ports = vec![22, 80];
        host.banners = vec![crate::host::ServiceBanner {
            port: 22,
            text: "SSH-2.0-test".into(),
        }];
        host
    }

    #[test]
    fn formulas_behind_a_tab_or_carriage_return_are_defused_too() {
        assert_eq!(escape_csv_field("=1+1"), "'=1+1");
        assert_eq!(escape_csv_field("\t=1+1"), "'\t=1+1");
        assert_eq!(escape_csv_field("\r+cmd"), "\"'\r+cmd\"");
        assert_eq!(escape_csv_field("plain"), "plain");
    }

    #[test]
    fn writes_header_and_quoted_row() {
        let csv = to_csv(&[sample_host()], &DeviceNotes::default());
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some(CSV_COLUMNS.join(",").as_str()));
        assert_eq!(
            lines.next(),
            Some(
                "192.168.0.2,1.5,\"nas, main\",AA:BB:CC:DD:EE:FF,,22 80,22: SSH-2.0-test,,,,,,,,,"
            )
        );
    }

    #[test]
    fn exports_include_the_users_notes() {
        let host = sample_host();
        let mut notes = DeviceNotes::default();
        notes.set(
            &host.device_key(),
            DeviceNote::from_input("Sam", "Attic", "lab", "=old"),
        );
        let csv = to_csv(std::slice::from_ref(&host), &notes);
        assert!(
            csv.lines()
                .nth(1)
                .unwrap()
                .ends_with(",Sam,Attic,lab,'=old")
        );
        let json = to_json(std::slice::from_ref(&host), &notes).unwrap();
        assert!(json.contains("\"owner\": \"Sam\""));
        let plain = to_json(&[host], &DeviceNotes::default()).unwrap();
        assert!(!plain.contains("\"note\""));
    }

    #[test]
    fn defuses_formula_injection() {
        let mut host = sample_host();
        host.hostname = Some("=HYPERLINK(\"http://evil\")".into());
        let csv = to_csv(&[host], &DeviceNotes::default());
        assert!(csv.contains("\"'=HYPERLINK(\"\"http://evil\"\")\""));
    }

    #[test]
    fn json_round_trips() {
        let hosts = vec![sample_host()];
        let json = to_json(&hosts, &DeviceNotes::default()).unwrap();
        let parsed: Vec<Host> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, hosts);
    }
}
