//! The rows of the results list, filtered and ordered once, when something changes, instead of
//! on every redraw.

use lanping_core::Host;
use lanping_core::device_notes::DeviceNotes;
use lanping_core::device_type::guess_device_type;
use lanping_core::diff::ScanDiff;
use lanping_core::labels::DeviceLabels;

use crate::host_filter::{HostFilter, RowStatus};
use crate::host_order::{SortKey, compare_hosts};
use crate::host_rows::Row;

/// Everything that decides which rows show, and in what order.
pub struct RowInputs<'a> {
    pub hosts: &'a [Host],
    pub diff: &'a ScanDiff,
    pub labels: &'a DeviceLabels,
    pub notes: &'a DeviceNotes,
    pub filter: &'a HostFilter,
    pub sort_key: SortKey,
    pub ascending: bool,
}

/// Where a row's host is kept.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Live(usize),
    Gone(usize),
}

#[derive(Debug, Clone)]
struct Entry {
    source: Source,
    status: RowStatus,
    /// The name the user gave the device.
    label: Option<String>,
}

#[derive(Debug, Default)]
pub struct RowList {
    entries: Vec<Entry>,
}

impl RowList {
    /// Works the list out again. Call it when the hosts, the changes, the filter, the order, or
    /// the names and notes have changed.
    pub fn rebuild(&mut self, inputs: &RowInputs<'_>) {
        self.entries = build(inputs);
    }

    /// The rows, live hosts first and gone ones last. `hosts` and `gone` are the lists the last
    /// [`RowList::rebuild`] was made from; a row whose host is no longer there is left out.
    pub fn rows<'a>(&'a self, hosts: &'a [Host], gone: &'a [Host]) -> Vec<Row<'a>> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let host = match entry.source {
                    Source::Live(index) => hosts.get(index),
                    Source::Gone(index) => gone.get(index),
                }?;
                Some(Row {
                    host,
                    status: entry.status,
                    label: entry.label.as_deref(),
                })
            })
            .collect()
    }
}

fn build(inputs: &RowInputs<'_>) -> Vec<Entry> {
    let needle = inputs.filter.needle();
    let live = inputs.hosts.iter().enumerate().map(|(index, host)| {
        (
            Source::Live(index),
            host,
            RowStatus::from(inputs.diff.status(host.ip)),
        )
    });
    let gone = inputs
        .diff
        .gone
        .iter()
        .enumerate()
        .map(|(index, host)| (Source::Gone(index), host, RowStatus::Gone));
    let mut entries: Vec<Entry> = live
        .chain(gone)
        .filter_map(|(source, host, status)| {
            let key = host.device_key();
            let label = inputs.labels.get(&key);
            let note = inputs.notes.get(&key);
            inputs
                .filter
                .matches_with(host, label, note, status, &needle)
                .then(|| Entry {
                    source,
                    status,
                    label: label.map(str::to_owned),
                })
        })
        .collect();
    order(&mut entries, inputs);
    entries
}

/// Live rows in the chosen order, then gone rows in the same order.
fn order(entries: &mut [Entry], inputs: &RowInputs<'_>) {
    let host_of = |entry: &Entry| match entry.source {
        Source::Live(index) => &inputs.hosts[index],
        Source::Gone(index) => &inputs.diff.gone[index],
    };
    if inputs.sort_key == SortKey::DeviceType {
        // The guess is worked out once per row, not once per comparison.
        entries.sort_by_cached_key(|entry| {
            let host = host_of(entry);
            (
                entry.status == RowStatus::Gone,
                guess_device_type(host),
                host.ip,
            )
        });
        if !inputs.ascending {
            let live = entries.partition_point(|entry| entry.status != RowStatus::Gone);
            entries[..live].reverse();
        }
        return;
    }
    entries.sort_by(|a, b| {
        let gone_last = (a.status == RowStatus::Gone).cmp(&(b.status == RowStatus::Gone));
        let order = compare_hosts(host_of(a), host_of(b), inputs.sort_key);
        gone_last.then(if inputs.ascending {
            order
        } else {
            order.reverse()
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_filter::FilterChange;
    use lanping_core::diff::diff_scans;

    fn host(last_octet: u8, name: &str) -> Host {
        let mut host = Host::new(std::net::Ipv4Addr::new(10, 0, 0, last_octet));
        host.mac = Some(format!("02:00:5E:10:00:{last_octet:02X}"));
        host.hostname = Some(name.to_owned());
        host
    }

    struct Fixture {
        hosts: Vec<Host>,
        diff: ScanDiff,
        labels: DeviceLabels,
        notes: DeviceNotes,
        filter: HostFilter,
    }

    impl Fixture {
        fn new(hosts: Vec<Host>) -> Self {
            Self {
                hosts,
                diff: ScanDiff::default(),
                labels: DeviceLabels::default(),
                notes: DeviceNotes::default(),
                filter: HostFilter::default(),
            }
        }

        fn names(&self, sort_key: SortKey, ascending: bool) -> Vec<String> {
            let mut list = RowList::default();
            list.rebuild(&RowInputs {
                hosts: &self.hosts,
                diff: &self.diff,
                labels: &self.labels,
                notes: &self.notes,
                filter: &self.filter,
                sort_key,
                ascending,
            });
            list.rows(&self.hosts, &self.diff.gone)
                .iter()
                .map(|row| row.host.hostname.clone().unwrap_or_default())
                .collect()
        }
    }

    fn three() -> Fixture {
        Fixture::new(vec![
            host(30, "charlie"),
            host(10, "Alpha"),
            host(20, "bravo"),
        ])
    }

    #[test]
    fn rows_follow_the_chosen_order_and_direction() {
        let fixture = three();
        assert_eq!(
            fixture.names(SortKey::Address, true),
            ["Alpha", "bravo", "charlie"]
        );
        assert_eq!(
            fixture.names(SortKey::Address, false),
            ["charlie", "bravo", "Alpha"]
        );
        assert_eq!(
            fixture.names(SortKey::Hostname, true),
            ["Alpha", "bravo", "charlie"]
        );
        assert_eq!(
            fixture.names(SortKey::Hostname, false),
            ["charlie", "bravo", "Alpha"]
        );
    }

    #[test]
    fn the_filter_narrows_the_rows_and_a_label_can_be_searched() {
        let mut fixture = three();
        fixture.filter.apply(FilterChange::Text("RAV".to_owned()));
        assert_eq!(fixture.names(SortKey::Address, true), ["bravo"]);
        fixture.labels.set("02:00:5E:10:00:1E", "kitchen printer");
        fixture
            .filter
            .apply(FilterChange::Text("kitchen".to_owned()));
        assert_eq!(fixture.names(SortKey::Address, true), ["charlie"]);
    }

    #[test]
    fn gone_devices_come_last_in_either_direction() {
        let before = vec![host(10, "alpha"), host(99, "old")];
        let mut fixture = Fixture::new(vec![host(10, "alpha"), host(20, "bravo")]);
        fixture.diff = diff_scans(&before, &fixture.hosts);
        assert_eq!(
            fixture.names(SortKey::Address, true),
            ["alpha", "bravo", "old"]
        );
        assert_eq!(
            fixture.names(SortKey::Address, false),
            ["bravo", "alpha", "old"]
        );
    }

    #[test]
    fn sorting_by_type_keeps_gone_last_and_reverses_only_the_live_rows() {
        let before = vec![host(10, "alpha"), host(99, "old")];
        let mut fixture = Fixture::new(vec![host(10, "alpha"), host(20, "bravo")]);
        fixture.diff = diff_scans(&before, &fixture.hosts);
        assert_eq!(
            fixture.names(SortKey::DeviceType, true),
            ["alpha", "bravo", "old"]
        );
        assert_eq!(
            fixture.names(SortKey::DeviceType, false),
            ["bravo", "alpha", "old"]
        );
    }

    #[test]
    fn rows_whose_host_has_gone_from_the_lists_are_left_out_instead_of_panicking() {
        let fixture = three();
        let mut list = RowList::default();
        list.rebuild(&RowInputs {
            hosts: &fixture.hosts,
            diff: &fixture.diff,
            labels: &fixture.labels,
            notes: &fixture.notes,
            filter: &fixture.filter,
            sort_key: SortKey::Address,
            ascending: true,
        });
        assert_eq!(list.rows(&fixture.hosts[..1], &[]).len(), 1);
        assert!(list.rows(&[], &[]).is_empty());
    }
}
