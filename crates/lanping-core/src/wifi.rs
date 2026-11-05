//! Nearby Wi-Fi access points, read through the system's own tools so no
//! root or admin rights are needed.
//!
//! Linux uses `nmcli`, Windows uses `netsh`. macOS is not supported yet: the
//! `airport` tool is gone and SSIDs are hidden without location permission.

use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::fmt;

/// Longest a scan may run before it is stopped.
#[cfg(any(target_os = "linux", windows))]
const SCAN_LIMIT: std::time::Duration = std::time::Duration::from_secs(20);

/// 2.4 GHz channels that do not overlap each other.
const CLEAR_CHANNELS_2_4: [u16; 3] = [1, 6, 11];
/// 5 GHz channels outside the DFS ranges, which routers may refuse to use.
const CLEAR_CHANNELS_5: [u16; 9] = [36, 40, 44, 48, 149, 153, 157, 161, 165];
/// 2.4 GHz channels closer than this share frequency space.
const OVERLAP_2_4: u16 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Band {
    Ghz2_4,
    Ghz5,
    Ghz6,
}

impl Band {
    pub const ALL: [Self; 3] = [Self::Ghz2_4, Self::Ghz5, Self::Ghz6];

    /// Band of a frequency in MHz.
    pub fn from_frequency(mhz: u32) -> Option<Self> {
        match mhz {
            2400..=2500 => Some(Self::Ghz2_4),
            5150..=5895 => Some(Self::Ghz5),
            5925..=7125 => Some(Self::Ghz6),
            _ => None,
        }
    }

    /// Center frequency in MHz of a channel in this band.
    fn frequency_of(self, channel: u16) -> u32 {
        let channel = u32::from(channel);
        match self {
            Self::Ghz2_4 if channel == 14 => 2484,
            Self::Ghz2_4 => 2407 + 5 * channel,
            Self::Ghz5 => 5000 + 5 * channel,
            Self::Ghz6 => 5950 + 5 * channel,
        }
    }
}

impl fmt::Display for Band {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ghz2_4 => "2.4 GHz",
            Self::Ghz5 => "5 GHz",
            Self::Ghz6 => "6 GHz",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPoint {
    /// Network name. Empty for a hidden network.
    pub ssid: String,
    /// Normalized MAC (`AA:BB:CC:DD:EE:FF`) of the radio.
    pub bssid: String,
    pub channel: u16,
    pub frequency_mhz: u32,
    pub band: Band,
    /// Signal quality from 0 to 100.
    pub signal_percent: u8,
    /// Empty for an open network.
    pub security: String,
    pub rate_mbps: Option<u32>,
}

/// How many access points use one channel and how strong the loudest one is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelLoad {
    pub band: Band,
    pub channel: u16,
    pub access_points: usize,
    pub strongest_percent: u8,
}

/// Why a scan produced nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiError {
    Unsupported,
    ToolMissing(&'static str),
    TimedOut,
    Failed(String),
}

impl fmt::Display for WifiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsupported => f.write_str("Wi-Fi scanning is not supported on this system yet"),
            Self::ToolMissing(tool) => write!(f, "Could not run {tool}. Is it installed?"),
            Self::TimedOut => f.write_str("The Wi-Fi scan took too long"),
            Self::Failed(reason) => f.write_str(reason),
        }
    }
}

/// Scans for nearby access points, strongest first.
pub async fn scan_access_points() -> Result<Vec<AccessPoint>, WifiError> {
    let mut access_points = read_access_points().await?;
    access_points.sort_by(strongest_first);
    Ok(access_points)
}

#[cfg(target_os = "linux")]
async fn read_access_points() -> Result<Vec<AccessPoint>, WifiError> {
    let args = [
        "-t",
        "-f",
        "SSID,BSSID,CHAN,FREQ,SIGNAL,SECURITY,RATE",
        "dev",
        "wifi",
        "list",
        "--rescan",
        "yes",
    ];
    run_tool("nmcli", &args)
        .await
        .map(|text| parse_nmcli(&text))
}

#[cfg(windows)]
async fn read_access_points() -> Result<Vec<AccessPoint>, WifiError> {
    run_tool("netsh", &["wlan", "show", "networks", "mode=bssid"])
        .await
        .map(|text| parse_netsh(&text))
}

#[cfg(not(any(target_os = "linux", windows)))]
async fn read_access_points() -> Result<Vec<AccessPoint>, WifiError> {
    Err(WifiError::Unsupported)
}

#[cfg(any(target_os = "linux", windows))]
async fn run_tool(tool: &'static str, args: &[&str]) -> Result<String, WifiError> {
    use tokio::process::Command;
    use tokio::time::timeout;

    let run = Command::new(tool).args(args).kill_on_drop(true).output();
    let output = timeout(SCAN_LIMIT, run)
        .await
        .map_err(|_| WifiError::TimedOut)?
        .map_err(|_| WifiError::ToolMissing(tool))?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    if output.status.success() {
        return Ok(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let reason = stderr.trim();
    Err(WifiError::Failed(if reason.is_empty() {
        format!("{tool} failed")
    } else {
        reason.to_owned()
    }))
}

fn strongest_first(a: &AccessPoint, b: &AccessPoint) -> Ordering {
    b.signal_percent
        .cmp(&a.signal_percent)
        .then_with(|| a.bssid.cmp(&b.bssid))
}

/// Parses `nmcli -t -f SSID,BSSID,CHAN,FREQ,SIGNAL,SECURITY,RATE dev wifi list`.
pub fn parse_nmcli(text: &str) -> Vec<AccessPoint> {
    text.lines().filter_map(parse_nmcli_line).collect()
}

fn parse_nmcli_line(line: &str) -> Option<AccessPoint> {
    let fields = split_terse(line);
    let [ssid, bssid, channel, frequency, signal, security, rate] = fields.as_slice() else {
        return None;
    };
    let frequency_mhz = first_number(frequency)?;
    Some(AccessPoint {
        ssid: ssid.clone(),
        bssid: normalize_bssid(bssid)?,
        channel: channel.trim().parse().ok()?,
        frequency_mhz,
        band: Band::from_frequency(frequency_mhz)?,
        signal_percent: signal.trim().parse::<u8>().ok()?.min(100),
        security: security.trim().trim_matches('-').to_owned(),
        rate_mbps: first_number(rate),
    })
}

/// Splits one line of `nmcli -t` output. `\:` and `\\` are escaped characters.
fn split_terse(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut characters = line.chars();
    while let Some(character) = characters.next() {
        match character {
            '\\' => current.extend(characters.next()),
            ':' => fields.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    fields.push(current);
    fields
}

fn first_number(text: &str) -> Option<u32> {
    text.split_whitespace().next()?.parse().ok()
}

fn normalize_bssid(raw: &str) -> Option<String> {
    let octets: Vec<String> = raw
        .trim()
        .split([':', '-'])
        .map(str::to_ascii_uppercase)
        .collect();
    let valid = octets.len() == 6
        && octets
            .iter()
            .all(|octet| octet.len() == 2 && octet.chars().all(|c| c.is_ascii_hexdigit()));
    valid.then(|| octets.join(":"))
}

/// Parses `netsh wlan show networks mode=bssid`. Only English output is understood.
pub fn parse_netsh(text: &str) -> Vec<AccessPoint> {
    let mut access_points = Vec::new();
    let mut network = NetshNetwork::default();
    let mut pending: Option<NetshRadio> = None;

    for line in text.lines() {
        let Some((key, value)) = line.split_once(" : ") else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let value = value.trim();
        if is_numbered(&key, "ssid") {
            finish_radio(&mut access_points, &network, pending.take());
            network = NetshNetwork {
                ssid: value.to_owned(),
                ..NetshNetwork::default()
            };
        } else if key == "authentication" {
            network.security = if value.eq_ignore_ascii_case("open") {
                String::new()
            } else {
                value.to_owned()
            };
        } else if is_numbered(&key, "bssid") {
            finish_radio(&mut access_points, &network, pending.take());
            pending = Some(NetshRadio {
                bssid: value.to_owned(),
                ..NetshRadio::default()
            });
        } else if let Some(radio) = pending.as_mut() {
            match key.as_str() {
                "signal" => radio.signal = value.trim_end_matches('%').trim().parse().ok(),
                "channel" => radio.channel = value.parse().ok(),
                "band" => radio.band = parse_band_name(value),
                _ => {}
            }
        }
    }
    finish_radio(&mut access_points, &network, pending.take());
    access_points
}

#[derive(Default)]
struct NetshNetwork {
    ssid: String,
    security: String,
}

#[derive(Default)]
struct NetshRadio {
    bssid: String,
    signal: Option<u8>,
    channel: Option<u16>,
    band: Option<Band>,
}

/// `ssid 3` or `bssid 1`, as opposed to a plain `ssid` key.
fn is_numbered(key: &str, name: &str) -> bool {
    key.strip_prefix(name)
        .is_some_and(|rest| rest.trim().parse::<u32>().is_ok())
}

fn parse_band_name(value: &str) -> Option<Band> {
    match value.split_whitespace().next()? {
        "2.4" => Some(Band::Ghz2_4),
        "5" => Some(Band::Ghz5),
        "6" => Some(Band::Ghz6),
        _ => None,
    }
}

fn finish_radio(
    access_points: &mut Vec<AccessPoint>,
    network: &NetshNetwork,
    radio: Option<NetshRadio>,
) {
    let Some(radio) = radio else {
        return;
    };
    let (Some(bssid), Some(channel), Some(signal)) =
        (normalize_bssid(&radio.bssid), radio.channel, radio.signal)
    else {
        return;
    };
    // Older Windows builds do not print the band. Channels above 14 only exist on 5 and 6 GHz.
    let band = radio.band.unwrap_or(if channel <= 14 {
        Band::Ghz2_4
    } else {
        Band::Ghz5
    });
    access_points.push(AccessPoint {
        ssid: network.ssid.clone(),
        bssid,
        channel,
        frequency_mhz: band.frequency_of(channel),
        band,
        signal_percent: signal.min(100),
        security: network.security.clone(),
        rate_mbps: None,
    });
}

/// Channels that have at least one access point, by band then channel.
pub fn channel_loads(access_points: &[AccessPoint]) -> Vec<ChannelLoad> {
    let mut loads: Vec<ChannelLoad> = Vec::new();
    for access_point in access_points {
        let existing = loads
            .iter_mut()
            .find(|load| load.band == access_point.band && load.channel == access_point.channel);
        match existing {
            Some(load) => {
                load.access_points += 1;
                load.strongest_percent = load.strongest_percent.max(access_point.signal_percent);
            }
            None => loads.push(ChannelLoad {
                band: access_point.band,
                channel: access_point.channel,
                access_points: 1,
                strongest_percent: access_point.signal_percent,
            }),
        }
    }
    loads.sort_by_key(|load| (load.band, load.channel));
    loads
}

/// The least crowded channel to move a router to. `None` for 6 GHz, which is rarely crowded.
pub fn recommend_channel(access_points: &[AccessPoint], band: Band) -> Option<u16> {
    let candidates: &[u16] = match band {
        Band::Ghz2_4 => &CLEAR_CHANNELS_2_4,
        Band::Ghz5 => &CLEAR_CHANNELS_5,
        Band::Ghz6 => return None,
    };
    candidates.iter().copied().min_by_key(|candidate| {
        access_points
            .iter()
            .filter(|access_point| {
                access_point.band == band && interferes(band, access_point.channel, *candidate)
            })
            .map(|access_point| u32::from(access_point.signal_percent))
            .sum::<u32>()
    })
}

fn interferes(band: Band, channel: u16, candidate: u16) -> bool {
    match band {
        Band::Ghz2_4 => channel.abs_diff(candidate) < OVERLAP_2_4,
        Band::Ghz5 | Band::Ghz6 => channel == candidate,
    }
}

/// Signal strength of each access point over the last scans.
///
/// Every scan adds one sample per known access point, so the samples of all of them
/// line up in time. A scan that did not see an access point leaves a gap (`None`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SignalHistory {
    samples: HashMap<String, VecDeque<Option<u8>>>,
    scans: usize,
}

impl SignalHistory {
    /// Samples kept per access point; older ones are dropped.
    pub const LENGTH: usize = 40;

    /// Adds one scan. Access points seen for the first time start with gaps.
    pub fn record(&mut self, access_points: &[AccessPoint]) {
        let earlier_scans = self.scans.min(Self::LENGTH);
        for access_point in access_points {
            self.samples
                .entry(access_point.bssid.clone())
                .or_insert_with(|| VecDeque::from(vec![None; earlier_scans]));
        }
        for (bssid, samples) in &mut self.samples {
            let signal = access_points
                .iter()
                .find(|access_point| access_point.bssid == *bssid)
                .map(|access_point| access_point.signal_percent);
            samples.push_back(signal);
            if samples.len() > Self::LENGTH {
                samples.pop_front();
            }
        }
        // An access point that has been out of range for the whole window is forgotten.
        self.samples
            .retain(|_, samples| samples.iter().any(Option::is_some));
        self.scans = (self.scans + 1).min(Self::LENGTH);
    }

    /// Oldest sample first. Empty for an access point that was never seen.
    pub fn samples(&self, bssid: &str) -> Vec<Option<u8>> {
        self.samples
            .get(bssid)
            .map(|samples| samples.iter().copied().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NMCLI: &str = "\
Tele2-2.4GHz:B2\\:9C\\:6C\\:D2\\:03\\:58:6:2437 MHz:87:WPA2:130 Mbit/s
:A8\\:9C\\:6C\\:D2\\:03\\:58:6:2437 MHz:84:WPA2:130 Mbit/s
TimeCapsule:B2\\:9C\\:6C\\:D2\\:03\\:59:36:5180 MHz:72:WPA2 WPA3:1170 Mbit/s
Guest\\:Net:9E\\:2A\\:6F\\:D4\\:78\\:56:44:5220 MHz:39::540 Mbit/s
";

    const NETSH: &str = "\
Interface name : Wi-Fi
There are 2 networks currently visible.

SSID 1 : HomeNet
    Network type            : Infrastructure
    Authentication          : WPA2-Personal
    Encryption              : CCMP
    BSSID 1                 : b2:9c:6c:d2:03:58
         Signal             : 91%
         Radio type         : 802.11n
         Band               : 2.4 GHz
         Channel            : 6
    BSSID 2                 : b2:9c:6c:d2:03:59
         Signal             : 60%
         Radio type         : 802.11ac
         Band               : 5 GHz
         Channel            : 36

SSID 2 : CoffeeShop
    Network type            : Infrastructure
    Authentication          : Open
    Encryption              : None
    BSSID 1                 : 9e:2a:6f:d4:78:55
         Signal             : 40%
         Radio type         : 802.11n
         Channel            : 11
";

    fn access_point(band: Band, channel: u16, signal_percent: u8) -> AccessPoint {
        AccessPoint {
            ssid: String::new(),
            bssid: "AA:BB:CC:DD:EE:FF".to_owned(),
            channel,
            frequency_mhz: band.frequency_of(channel),
            band,
            signal_percent,
            security: String::new(),
            rate_mbps: None,
        }
    }

    #[test]
    fn parses_nmcli_rows() {
        let parsed = parse_nmcli(NMCLI);
        assert_eq!(parsed.len(), 4);
        assert_eq!(parsed[0].ssid, "Tele2-2.4GHz");
        assert_eq!(parsed[0].bssid, "B2:9C:6C:D2:03:58");
        assert_eq!(parsed[0].channel, 6);
        assert_eq!(parsed[0].frequency_mhz, 2437);
        assert_eq!(parsed[0].band, Band::Ghz2_4);
        assert_eq!(parsed[0].signal_percent, 87);
        assert_eq!(parsed[0].security, "WPA2");
        assert_eq!(parsed[0].rate_mbps, Some(130));
    }

    #[test]
    fn nmcli_keeps_hidden_and_escaped_names() {
        let parsed = parse_nmcli(NMCLI);
        assert_eq!(parsed[1].ssid, "");
        assert_eq!(parsed[3].ssid, "Guest:Net");
    }

    #[test]
    fn nmcli_security_may_be_open_or_combined() {
        let parsed = parse_nmcli(NMCLI);
        assert_eq!(parsed[2].security, "WPA2 WPA3");
        assert_eq!(parsed[3].security, "");
        assert_eq!(parsed[2].band, Band::Ghz5);
    }

    #[test]
    fn nmcli_skips_malformed_lines() {
        assert!(parse_nmcli("garbage\n\nSSID:not-a-mac:1:2437 MHz:50:WPA2:1 Mbit/s").is_empty());
    }

    #[test]
    fn parses_netsh_networks_with_several_radios() {
        let parsed = parse_netsh(NETSH);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].ssid, "HomeNet");
        assert_eq!(parsed[0].bssid, "B2:9C:6C:D2:03:58");
        assert_eq!(parsed[0].security, "WPA2-Personal");
        assert_eq!(parsed[1].band, Band::Ghz5);
        assert_eq!(parsed[1].frequency_mhz, 5180);
        assert_eq!(parsed[1].ssid, "HomeNet");
    }

    #[test]
    fn netsh_open_network_has_no_security_and_band_comes_from_channel() {
        let parsed = parse_netsh(NETSH);
        assert_eq!(parsed[2].ssid, "CoffeeShop");
        assert_eq!(parsed[2].security, "");
        assert_eq!(parsed[2].band, Band::Ghz2_4);
        assert_eq!(parsed[2].frequency_mhz, 2462);
    }

    #[test]
    fn netsh_with_no_networks_is_empty() {
        assert!(parse_netsh("There are 0 networks currently visible.").is_empty());
    }

    #[test]
    fn groups_access_points_by_channel() {
        let loads = channel_loads(&[
            access_point(Band::Ghz2_4, 6, 40),
            access_point(Band::Ghz2_4, 6, 80),
            access_point(Band::Ghz5, 36, 50),
        ]);
        assert_eq!(loads.len(), 2);
        assert_eq!(loads[0].channel, 6);
        assert_eq!(loads[0].access_points, 2);
        assert_eq!(loads[0].strongest_percent, 80);
        assert_eq!(loads[1].band, Band::Ghz5);
    }

    #[test]
    fn recommends_the_quietest_clear_2_4_channel() {
        let crowded = [
            access_point(Band::Ghz2_4, 1, 90),
            access_point(Band::Ghz2_4, 6, 70),
            access_point(Band::Ghz2_4, 3, 50),
        ];
        assert_eq!(recommend_channel(&crowded, Band::Ghz2_4), Some(11));
    }

    #[test]
    fn recommends_the_first_clear_channel_when_nothing_is_nearby() {
        assert_eq!(recommend_channel(&[], Band::Ghz2_4), Some(1));
        assert_eq!(recommend_channel(&[], Band::Ghz5), Some(36));
    }

    #[test]
    fn five_ghz_channels_do_not_overlap() {
        let busy = [access_point(Band::Ghz5, 36, 90)];
        assert_eq!(recommend_channel(&busy, Band::Ghz5), Some(40));
    }

    #[test]
    fn six_ghz_has_no_recommendation() {
        assert_eq!(recommend_channel(&[], Band::Ghz6), None);
    }

    #[test]
    fn maps_frequencies_to_bands() {
        assert_eq!(Band::from_frequency(2437), Some(Band::Ghz2_4));
        assert_eq!(Band::from_frequency(5180), Some(Band::Ghz5));
        assert_eq!(Band::from_frequency(5955), Some(Band::Ghz6));
        assert_eq!(Band::from_frequency(900), None);
    }

    #[test]
    fn history_lines_up_samples_of_every_access_point() {
        let mut history = SignalHistory::default();
        let first = access_point(Band::Ghz2_4, 6, 80);
        let mut second = access_point(Band::Ghz5, 36, 50);
        second.bssid = "11:22:33:44:55:66".to_owned();
        history.record(std::slice::from_ref(&first));
        history.record(&[first.clone(), second.clone()]);
        history.record(std::slice::from_ref(&second));
        assert_eq!(history.samples(&first.bssid), [Some(80), Some(80), None]);
        assert_eq!(history.samples(&second.bssid), [None, Some(50), Some(50)]);
    }

    #[test]
    fn history_keeps_only_the_latest_samples() {
        let mut history = SignalHistory::default();
        let access_point = access_point(Band::Ghz2_4, 6, 80);
        for _ in 0..SignalHistory::LENGTH + 5 {
            history.record(std::slice::from_ref(&access_point));
        }
        assert_eq!(
            history.samples(&access_point.bssid).len(),
            SignalHistory::LENGTH
        );
    }

    #[test]
    fn history_forgets_access_points_gone_for_the_whole_window() {
        let mut history = SignalHistory::default();
        let gone = access_point(Band::Ghz2_4, 6, 80);
        history.record(std::slice::from_ref(&gone));
        for _ in 0..SignalHistory::LENGTH {
            history.record(&[]);
        }
        assert!(history.samples(&gone.bssid).is_empty());
    }

    #[test]
    fn history_of_an_unknown_access_point_is_empty() {
        assert!(SignalHistory::default().samples("nope").is_empty());
    }
}
