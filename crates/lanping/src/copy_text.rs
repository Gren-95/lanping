//! Plain-text versions of the host list, for the clipboard.

use lanping_core::device_type::guess_device_type;
use lanping_core::ports::format_ports;

use crate::host_rows::Row;
use crate::settings::Column;

const NO_VALUE: &str = "-";

/// Tab-separated rows with a header line, for the shown columns only.
/// The "Open with" column has no text and is skipped.
pub fn rows_as_text(rows: &[Row<'_>], columns: &[Column]) -> String {
    let columns: Vec<Column> = columns
        .iter()
        .copied()
        .filter(|&column| column != Column::Actions)
        .collect();
    let header = columns
        .iter()
        .map(|column| column.title())
        .collect::<Vec<_>>()
        .join("\t");
    let lines = rows.iter().map(|row| {
        columns
            .iter()
            .map(|&column| cell_text(column, row))
            .collect::<Vec<_>>()
            .join("\t")
    });
    std::iter::once(header)
        .chain(lines)
        .collect::<Vec<_>>()
        .join("\n")
}

fn cell_text(column: Column, row: &Row<'_>) -> String {
    let host = row.host;
    match column {
        Column::Address => host.ip.to_string(),
        Column::Latency => host.latency.map_or_else(
            || NO_VALUE.to_owned(),
            |latency| format!("{:.1} ms", latency.as_secs_f64() * 1000.0),
        ),
        Column::Hostname => row
            .label
            .map(str::to_owned)
            .or_else(|| host.display_name().map(str::to_owned))
            .unwrap_or_else(|| NO_VALUE.to_owned()),
        Column::Mac => host.mac.clone().unwrap_or_else(|| NO_VALUE.to_owned()),
        Column::Vendor => host.vendor.clone().unwrap_or_else(|| NO_VALUE.to_owned()),
        Column::DeviceType => guess_device_type(host).to_string(),
        Column::Ports if host.open_ports.is_empty() => NO_VALUE.to_owned(),
        Column::Ports => format_ports(&host.open_ports),
        Column::Actions => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use lanping_core::Host;

    use super::*;
    use crate::host_filter::RowStatus;

    #[test]
    fn copies_a_header_and_one_line_per_row() {
        let mut host = Host::new(Ipv4Addr::new(10, 0, 0, 7));
        host.latency = Some(Duration::from_millis(12));
        host.open_ports = vec![22, 80];
        let bare = Host::new(Ipv4Addr::new(10, 0, 0, 8));
        let rows = [
            Row {
                host: &host,
                status: RowStatus::Unchanged,
                label: Some("NAS"),
            },
            Row {
                host: &bare,
                status: RowStatus::New,
                label: None,
            },
        ];
        let text = rows_as_text(
            &rows,
            &[
                Column::Address,
                Column::Latency,
                Column::Hostname,
                Column::Ports,
                Column::Actions,
            ],
        );
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Address\tLatency\tHostname\tOpen ports");
        assert_eq!(lines[1], "10.0.0.7\t12.0 ms\tNAS\t22, 80");
        assert_eq!(lines[2], "10.0.0.8\t-\t-\t-");
    }
}
