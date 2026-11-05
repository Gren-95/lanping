//! Searching, filtering and ordering of the access point list, and its text form.

use std::cmp::Ordering;
use std::fmt;

use lanping_core::vendor::lookup_vendor;
use lanping_core::wifi::{AccessPoint, Band};

const NO_VALUE: &str = "-";
const OPEN_LABEL: &str = "Open";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WifiSortKey {
    Network,
    Bssid,
    Vendor,
    Channel,
    Band,
    #[default]
    Signal,
    Security,
    Rate,
}

impl WifiSortKey {
    pub const ALL: [Self; 8] = [
        Self::Network,
        Self::Bssid,
        Self::Vendor,
        Self::Channel,
        Self::Band,
        Self::Signal,
        Self::Security,
        Self::Rate,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Network => "Network",
            Self::Bssid => "BSSID",
            Self::Vendor => "Vendor",
            Self::Channel => "Channel",
            Self::Band => "Band",
            Self::Signal => "Signal",
            Self::Security => "Security",
            Self::Rate => "Rate",
        }
    }

    fn compare(self, a: &AccessPoint, b: &AccessPoint) -> Ordering {
        let by_bssid = a.bssid.cmp(&b.bssid);
        match self {
            Self::Network => compare_text(&a.ssid, &b.ssid).then(by_bssid),
            Self::Bssid => by_bssid,
            Self::Vendor => {
                compare_text(vendor_of(a).unwrap_or(""), vendor_of(b).unwrap_or("")).then(by_bssid)
            }
            Self::Channel => (a.band, a.channel).cmp(&(b.band, b.channel)).then(by_bssid),
            Self::Band => a.band.cmp(&b.band).then(by_bssid),
            Self::Signal => a.signal_percent.cmp(&b.signal_percent).then(by_bssid),
            Self::Security => compare_text(&a.security, &b.security).then(by_bssid),
            Self::Rate => a.rate_mbps.cmp(&b.rate_mbps).then(by_bssid),
        }
    }
}

/// Case-insensitive, with empty text (hidden names, no vendor) after everything else.
fn compare_text(a: &str, b: &str) -> Ordering {
    a.is_empty()
        .cmp(&b.is_empty())
        .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
}

fn vendor_of(access_point: &AccessPoint) -> Option<&'static str> {
    lookup_vendor(&access_point.bssid)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BandChoice {
    #[default]
    Any,
    Only(Band),
}

impl BandChoice {
    pub const ALL: [Self; 4] = [
        Self::Any,
        Self::Only(Band::Ghz2_4),
        Self::Only(Band::Ghz5),
        Self::Only(Band::Ghz6),
    ];
}

impl fmt::Display for BandChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Any => f.write_str("Any band"),
            Self::Only(band) => band.fmt(f),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SecurityChoice {
    #[default]
    Any,
    Open,
    Secured,
}

impl SecurityChoice {
    pub const ALL: [Self; 3] = [Self::Any, Self::Open, Self::Secured];
}

impl fmt::Display for SecurityChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Any => "Any security",
            Self::Open => "Open only",
            Self::Secured => "Secured only",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiFilterChange {
    Text(String),
    Band(BandChoice),
    Security(SecurityChoice),
    Clear,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WifiFilter {
    pub text: String,
    pub band: BandChoice,
    pub security: SecurityChoice,
}

impl WifiFilter {
    pub fn apply(&mut self, change: WifiFilterChange) {
        match change {
            WifiFilterChange::Text(text) => self.text = text,
            WifiFilterChange::Band(band) => self.band = band,
            WifiFilterChange::Security(security) => self.security = security,
            WifiFilterChange::Clear => *self = Self::default(),
        }
    }

    /// How many of the controls narrow the list.
    pub fn active_count(&self) -> usize {
        usize::from(!self.text.trim().is_empty())
            + usize::from(self.band != BandChoice::Any)
            + usize::from(self.security != SecurityChoice::Any)
    }

    fn matches(&self, access_point: &AccessPoint) -> bool {
        let band_matches = match self.band {
            BandChoice::Any => true,
            BandChoice::Only(band) => access_point.band == band,
        };
        let security_matches = match self.security {
            SecurityChoice::Any => true,
            SecurityChoice::Open => access_point.security.is_empty(),
            SecurityChoice::Secured => !access_point.security.is_empty(),
        };
        band_matches && security_matches && self.text_matches(access_point)
    }

    fn text_matches(&self, access_point: &AccessPoint) -> bool {
        let needle = self.text.trim().to_lowercase();
        if needle.is_empty() {
            return true;
        }
        let fields = [
            access_point.ssid.clone(),
            access_point.bssid.clone(),
            vendor_of(access_point).unwrap_or_default().to_owned(),
            access_point.security.clone(),
            access_point.channel.to_string(),
            access_point.band.to_string(),
        ];
        fields
            .iter()
            .any(|field| field.to_lowercase().contains(&needle))
    }
}

/// The access points that pass `filter`, in the requested order.
pub fn visible<'a>(
    access_points: &'a [AccessPoint],
    filter: &WifiFilter,
    key: WifiSortKey,
    ascending: bool,
) -> Vec<&'a AccessPoint> {
    let mut rows: Vec<&AccessPoint> = access_points
        .iter()
        .filter(|access_point| filter.matches(access_point))
        .collect();
    rows.sort_by(|a, b| {
        let order = key.compare(a, b);
        if ascending { order } else { order.reverse() }
    });
    rows
}

/// Tab-separated rows with a header line, ready to paste into a spreadsheet.
pub fn rows_as_text(rows: &[&AccessPoint]) -> String {
    let header = WifiSortKey::ALL
        .iter()
        .map(|key| key.title())
        .collect::<Vec<_>>()
        .join("\t");
    let lines = rows.iter().map(|access_point| {
        [
            access_point.ssid.clone(),
            access_point.bssid.clone(),
            vendor_of(access_point).unwrap_or(NO_VALUE).to_owned(),
            access_point.channel.to_string(),
            access_point.band.to_string(),
            format!("{}%", access_point.signal_percent),
            if access_point.security.is_empty() {
                OPEN_LABEL.to_owned()
            } else {
                access_point.security.clone()
            },
            access_point
                .rate_mbps
                .map_or_else(|| NO_VALUE.to_owned(), |rate| format!("{rate} Mbit/s")),
        ]
        .join("\t")
    });
    std::iter::once(header)
        .chain(lines)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn access_point(ssid: &str, bssid: &str, band: Band, channel: u16, signal: u8) -> AccessPoint {
        AccessPoint {
            ssid: ssid.to_owned(),
            bssid: bssid.to_owned(),
            channel,
            frequency_mhz: 0,
            band,
            signal_percent: signal,
            security: "WPA2".to_owned(),
            rate_mbps: Some(130),
        }
    }

    fn sample() -> Vec<AccessPoint> {
        let mut open = access_point("Cafe", "02:00:00:00:00:03", Band::Ghz2_4, 11, 40);
        open.security.clear();
        open.rate_mbps = None;
        vec![
            access_point("home", "02:00:00:00:00:01", Band::Ghz2_4, 6, 90),
            access_point("Attic", "02:00:00:00:00:02", Band::Ghz5, 36, 60),
            open,
            access_point("", "02:00:00:00:00:04", Band::Ghz5, 44, 20),
        ]
    }

    /// Names of the sample access points that pass `filter`, in the requested order.
    fn shown(filter: &WifiFilter, key: WifiSortKey, ascending: bool) -> Vec<String> {
        let access_points = sample();
        visible(&access_points, filter, key, ascending)
            .iter()
            .map(|row| row.ssid.clone())
            .collect()
    }

    #[test]
    fn default_order_is_strongest_signal_first() {
        let order = shown(&WifiFilter::default(), WifiSortKey::Signal, false);
        assert_eq!(order, ["home", "Attic", "Cafe", ""]);
    }

    #[test]
    fn network_sort_ignores_case_and_puts_hidden_last() {
        let order = shown(&WifiFilter::default(), WifiSortKey::Network, true);
        assert_eq!(order, ["Attic", "Cafe", "home", ""]);
    }

    #[test]
    fn descending_reverses_the_order() {
        let order = shown(&WifiFilter::default(), WifiSortKey::Channel, false);
        assert_eq!(order, ["", "Attic", "Cafe", "home"]);
    }

    #[test]
    fn rate_sort_places_unknown_rates_first_when_ascending() {
        let order = shown(&WifiFilter::default(), WifiSortKey::Rate, true);
        assert_eq!(order[0], "Cafe");
    }

    #[test]
    fn text_search_matches_name_bssid_and_channel() {
        let mut filter = WifiFilter::default();
        filter.apply(WifiFilterChange::Text("ATT".to_owned()));
        assert_eq!(shown(&filter, WifiSortKey::Signal, false), ["Attic"]);
        filter.apply(WifiFilterChange::Text("00:03".to_owned()));
        assert_eq!(shown(&filter, WifiSortKey::Signal, false), ["Cafe"]);
        filter.apply(WifiFilterChange::Text("44".to_owned()));
        assert_eq!(shown(&filter, WifiSortKey::Signal, false), [""]);
    }

    #[test]
    fn band_and_security_filters_combine() {
        let mut filter = WifiFilter::default();
        filter.apply(WifiFilterChange::Band(BandChoice::Only(Band::Ghz2_4)));
        filter.apply(WifiFilterChange::Security(SecurityChoice::Open));
        assert_eq!(shown(&filter, WifiSortKey::Signal, false), ["Cafe"]);
        assert_eq!(filter.active_count(), 2);
    }

    #[test]
    fn clearing_resets_every_filter() {
        let mut filter = WifiFilter::default();
        filter.apply(WifiFilterChange::Text("x".to_owned()));
        filter.apply(WifiFilterChange::Band(BandChoice::Only(Band::Ghz5)));
        filter.apply(WifiFilterChange::Clear);
        assert_eq!(filter, WifiFilter::default());
        assert_eq!(filter.active_count(), 0);
    }

    #[test]
    fn blank_search_does_not_count_as_active() {
        let mut filter = WifiFilter::default();
        filter.apply(WifiFilterChange::Text("   ".to_owned()));
        assert_eq!(filter.active_count(), 0);
    }

    #[test]
    fn text_form_has_a_header_and_one_line_per_row() {
        let sample = sample();
        let rows = visible(&sample, &WifiFilter::default(), WifiSortKey::Signal, false);
        let text = rows_as_text(&rows);
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 5);
        assert_eq!(
            lines[0],
            "Network\tBSSID\tVendor\tChannel\tBand\tSignal\tSecurity\tRate"
        );
        assert_eq!(
            lines[1],
            "home\t02:00:00:00:00:01\t-\t6\t2.4 GHz\t90%\tWPA2\t130 Mbit/s"
        );
        assert!(text.contains("Open\t-"));
    }
}
