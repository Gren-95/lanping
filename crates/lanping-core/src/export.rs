use serde::Serialize;

use crate::device_notes::{DeviceNote, DeviceNotes};
use crate::host::Host;

const CSV_HEADER: &str = "ip,latency_ms,hostname,mac,vendor,open_ports,banners,ipv6,device_name,manufacturer,model,services,owner,location,tags,notes";

pub fn to_csv(hosts: &[Host], notes: &DeviceNotes) -> String {
    let mut output = String::from(CSV_HEADER);
    output.push('\n');
    for host in hosts {
        let latency = host
            .latency
            .map(|latency| format!("{:.1}", latency.as_secs_f64() * 1000.0))
            .unwrap_or_default();
        let ports = host
            .open_ports
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let banners = host
            .banners
            .iter()
            .map(|banner| format!("{}: {}", banner.port, banner.text))
            .collect::<Vec<_>>()
            .join("; ");
        let ipv6 = host
            .ipv6
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ");
        let note = notes.get(&host.device_key()).cloned().unwrap_or_default();
        let fields = [
            host.ip.to_string(),
            latency,
            host.hostname.clone().unwrap_or_default(),
            host.mac.clone().unwrap_or_default(),
            host.vendor.clone().unwrap_or_default(),
            ports,
            banners,
            ipv6,
            host.discovery.name.clone().unwrap_or_default(),
            host.discovery.manufacturer.clone().unwrap_or_default(),
            host.discovery.model.clone().unwrap_or_default(),
            host.discovery.services.join(" "),
            note.owner.clone(),
            note.location.clone(),
            note.tags.join(" "),
            note.notes,
        ];
        let row: Vec<String> = fields.iter().map(|field| escape_csv_field(field)).collect();
        output.push_str(&row.join(","));
        output.push('\n');
    }
    output
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

/// Quotes fields that need it (RFC 4180) and defuses spreadsheet formulas
/// by prefixing a quote to values that start with `= + - @`.
fn escape_csv_field(field: &str) -> String {
    let defused = if field.starts_with(['=', '+', '-', '@']) {
        format!("'{field}")
    } else {
        field.to_owned()
    };
    if defused.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", defused.replace('"', "\"\""))
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
    fn writes_header_and_quoted_row() {
        let csv = to_csv(&[sample_host()], &DeviceNotes::default());
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some(CSV_HEADER));
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
