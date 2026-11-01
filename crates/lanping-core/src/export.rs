use crate::host::Host;

const CSV_HEADER: &str = "ip,latency_ms,hostname,mac,vendor,open_ports,banners";

pub fn to_csv(hosts: &[Host]) -> String {
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
        let fields = [
            host.ip.to_string(),
            latency,
            host.hostname.clone().unwrap_or_default(),
            host.mac.clone().unwrap_or_default(),
            host.vendor.clone().unwrap_or_default(),
            ports,
            banners,
        ];
        let row: Vec<String> = fields.iter().map(|field| escape_csv_field(field)).collect();
        output.push_str(&row.join(","));
        output.push('\n');
    }
    output
}

pub fn to_json(hosts: &[Host]) -> serde_json::Result<String> {
    serde_json::to_string_pretty(hosts)
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
        host.banners = vec![crate::host::ServiceBanner { port: 22, text: "SSH-2.0-test".into() }];
        host
    }

    #[test]
    fn writes_header_and_quoted_row() {
        let csv = to_csv(&[sample_host()]);
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some(CSV_HEADER));
        assert_eq!(
            lines.next(),
            Some("192.168.0.2,1.5,\"nas, main\",AA:BB:CC:DD:EE:FF,,22 80,22: SSH-2.0-test")
        );
    }

    #[test]
    fn defuses_formula_injection() {
        let mut host = sample_host();
        host.hostname = Some("=HYPERLINK(\"http://evil\")".into());
        let csv = to_csv(&[host]);
        assert!(csv.contains("\"'=HYPERLINK(\"\"http://evil\"\")\""));
    }

    #[test]
    fn json_round_trips() {
        let hosts = vec![sample_host()];
        let json = to_json(&hosts).unwrap();
        let parsed: Vec<Host> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, hosts);
    }
}
