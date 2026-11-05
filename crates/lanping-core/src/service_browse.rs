//! What announces itself on the local network: printers, Chromecasts, AirPlay, file shares,
//! media servers. Found by asking the whole network over multicast DNS and SSDP, which is
//! the same question any phone or TV asks.
//!
//! The mDNS questions leave from an ordinary port, so devices answer this computer directly
//! and nothing needs to be joined or bound on the shared mDNS port.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::mdns::{
    HEADER_LENGTH, LOCAL_SUFFIX, MDNS_PORT, RECORD_PTR, SERVICE_LIST_NAME, query_packet, read_name,
    read_u16,
};
use crate::snmp::clean_text;
use crate::socket_budget::socket_permit;
use crate::ssdp::{announcements, friendly_name};

const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
const RECORD_A: u16 = 1;
const RECORD_TXT: u16 = 16;
const RECORD_SRV: u16 = 33;
/// Header bytes of a resource record after its name: type, class, time to live, data length.
const RECORD_FIXED_LENGTH: usize = 10;
const SRV_FIXED_LENGTH: usize = 6;
const TYPE_LIST_WAIT: Duration = Duration::from_millis(1500);
const INSTANCE_WAIT: Duration = Duration::from_millis(2500);
const SSDP_WAIT: Duration = Duration::from_millis(2000);
const QUERY_ROUNDS: usize = 2;
const RECEIVE_BUFFER: usize = 9000;
const MAX_TYPES: usize = 64;
const MAX_INSTANCES: usize = 512;
const MAX_TEXT_ENTRIES: usize = 16;
const MAX_TEXT_CHARS: usize = 200;
const MAX_UPNP_DESCRIPTIONS: usize = 64;
const SUBTYPE_MARKER: &str = "._sub.";
const UPNP_KIND_PREFIX: &str = "upnp:";
const HARDWARE_ADDRESS_DIGITS: usize = 12;
const UUID_LENGTH: usize = 36;
const UUID_DASHES: [usize; 4] = [8, 13, 18, 23];
/// Text records longer than this are keys or hashes, not something to read.
const MAX_READABLE_CHARS: usize = 40;
const MAX_READABLE_ENTRIES: usize = 8;

/// One thing found on the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceEntry {
    /// The kind: `_ipp._tcp` for mDNS, `upnp:MediaServer` for UPnP.
    pub kind: String,
    pub name: String,
    pub address: Option<Ipv4Addr>,
    pub port: Option<u16>,
    /// The host name the device gave for itself, without `.local`.
    pub host: Option<String>,
    /// The text records, or for UPnP where the description is.
    pub details: Vec<String>,
}

impl ServiceEntry {
    /// A name a person can read. Some devices name their services after a hardware address
    /// or an identifier; then the host name, or failing that the address, says more.
    pub fn display_name(&self) -> String {
        let name = strip_hardware_prefix(&self.name);
        if !looks_machine_made(name) {
            return name.to_owned();
        }
        self.host
            .clone()
            .or_else(|| self.address.map(|address| address.to_string()))
            .unwrap_or_else(|| name.to_owned())
    }

    /// The text records worth reading: the short ones with a value. Keys, hashes and
    /// feature masks are left out. Also how many were left out.
    pub fn readable_details(&self) -> (Vec<&str>, usize) {
        let readable = |entry: &&String| {
            entry
                .split_once('=')
                .is_none_or(|(_, value)| !value.is_empty())
                && entry.chars().count() <= MAX_READABLE_CHARS
        };
        let shown: Vec<&str> = self
            .details
            .iter()
            .filter(readable)
            .map(String::as_str)
            .take(MAX_READABLE_ENTRIES)
            .collect();
        let hidden = self.details.len() - shown.len();
        (shown, hidden)
    }
}

/// `9E74262E9FCB@Living room` becomes `Living room`.
fn strip_hardware_prefix(name: &str) -> &str {
    match name.split_once('@') {
        Some((prefix, rest))
            if prefix.len() == HARDWARE_ADDRESS_DIGITS
                && prefix.chars().all(|c| c.is_ascii_hexdigit())
                && !rest.is_empty() =>
        {
            rest
        }
        _ => name,
    }
}

/// An identifier or a hardware and IPv6 address pair, not a name anyone chose.
fn looks_machine_made(name: &str) -> bool {
    name.contains("@fe80") || is_uuid(name)
}

fn is_uuid(text: &str) -> bool {
    text.len() == UUID_LENGTH
        && text.char_indices().all(|(at, c)| {
            if UUID_DASHES.contains(&at) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}

/// A plain name for a kind, such as `Printers` for `_ipp._tcp`. A kind with no plain name is
/// tidied up: `_apple-mobdev2._tcp` becomes `apple-mobdev2 (tcp)`.
pub fn kind_title(kind: &str) -> String {
    if let Some(device) = kind.strip_prefix(UPNP_KIND_PREFIX) {
        return format!("UPnP {device}");
    }
    let known = match kind {
        "_ipp._tcp" | "_printer._tcp" | "_pdl-datastream._tcp" => "Printers",
        "_airplay._tcp" => "AirPlay",
        "_raop._tcp" => "AirPlay audio",
        "_googlecast._tcp" => "Chromecast and Google Cast",
        "_smb._tcp" => "File shares (SMB)",
        "_afpovertcp._tcp" => "File shares (AFP)",
        "_nfs._tcp" => "File shares (NFS)",
        "_http._tcp" => "Web pages",
        "_https._tcp" => "Secure web pages",
        "_ssh._tcp" | "_sftp-ssh._tcp" => "SSH",
        "_homekit._tcp" | "_hap._tcp" => "HomeKit",
        "_spotify-connect._tcp" => "Spotify Connect",
        "_workstation._tcp" => "Computers",
        "_device-info._tcp" => "Device information",
        "_apple-mobdev2._tcp" => "iPhone and iPad sync",
        "_companion-link._tcp" => "Apple device link",
        "_remotepairing._tcp" => "Apple remote pairing",
        "_asquic._udp" => "Apple background transfer",
        "_meshcop._udp" => "Thread border routers",
        "_glinet._tcp" => "GL.iNet devices",
        _ => return tidy_kind(kind),
    };
    known.to_owned()
}

fn tidy_kind(kind: &str) -> String {
    match kind
        .strip_prefix('_')
        .and_then(|rest| rest.split_once("._"))
    {
        Some((service, protocol)) => format!("{service} ({protocol})"),
        None => kind.to_owned(),
    }
}

enum Data {
    Pointer(String),
    Service { port: u16, target: String },
    Text(Vec<String>),
    Address(Ipv4Addr),
    Other,
}

struct Record {
    name: String,
    data: Data,
}

/// Every record of a response, from the answers, authority and additional sections alike.
/// Stops at the first one that does not fit the packet.
fn parse_records(packet: &[u8]) -> Vec<Record> {
    let mut records = Vec::new();
    let counts = [4, 6, 8, 10].map(|offset| read_u16(packet, offset));
    let [
        Some(questions),
        Some(answers),
        Some(authority),
        Some(additional),
    ] = counts
    else {
        return records;
    };
    let mut position = HEADER_LENGTH;
    for _ in 0..questions {
        let Some((_, after)) = read_name(packet, position) else {
            return records;
        };
        position = after + 4;
    }
    let total = usize::from(answers) + usize::from(authority) + usize::from(additional);
    for _ in 0..total {
        let Some((name, after)) = read_name(packet, position) else {
            break;
        };
        let (Some(kind), Some(length)) = (read_u16(packet, after), read_u16(packet, after + 8))
        else {
            break;
        };
        let start = after + RECORD_FIXED_LENGTH;
        let end = start + usize::from(length);
        if end > packet.len() {
            break;
        }
        records.push(Record {
            name,
            data: parse_data(packet, kind, start, end),
        });
        position = end;
    }
    records
}

fn parse_data(packet: &[u8], kind: u16, start: usize, end: usize) -> Data {
    match kind {
        RECORD_PTR => {
            read_name(packet, start).map_or(Data::Other, |(target, _)| Data::Pointer(target))
        }
        RECORD_SRV => {
            let (Some(port), Some((target, _))) = (
                read_u16(packet, start + 4),
                read_name(packet, start + SRV_FIXED_LENGTH),
            ) else {
                return Data::Other;
            };
            Data::Service { port, target }
        }
        RECORD_TXT => Data::Text(parse_text(&packet[start..end])),
        RECORD_A => match <[u8; 4]>::try_from(&packet[start..end]) {
            Ok(octets) => Data::Address(Ipv4Addr::from(octets)),
            Err(_) => Data::Other,
        },
        _ => Data::Other,
    }
}

/// The strings of a TXT record, each one length-prefixed.
fn parse_text(data: &[u8]) -> Vec<String> {
    let mut entries = Vec::new();
    let mut rest = data;
    while let Some((&length, tail)) = rest.split_first() {
        let length = usize::from(length);
        if length > tail.len() || entries.len() >= MAX_TEXT_ENTRIES {
            break;
        }
        let (entry, after) = tail.split_at(length);
        if let Some(text) = text_entry(entry) {
            entries.push(text.chars().take(MAX_TEXT_CHARS).collect());
        }
        rest = after;
    }
    entries
}

/// One `key=value` string. A value that is not text, such as a key or an address in raw
/// bytes, is shown as `key=(binary)` instead of as garbage.
fn text_entry(entry: &[u8]) -> Option<String> {
    if std::str::from_utf8(entry).is_ok() {
        return clean_text(entry);
    }
    let key = entry.split(|byte| *byte == b'=').next()?;
    let key = std::str::from_utf8(key)
        .ok()
        .filter(|key| !key.is_empty())?;
    clean_text(format!("{key}=(binary)").as_bytes())
}

/// What the mDNS answers have said so far.
#[derive(Default)]
struct Directory {
    /// Service types, such as `_ipp._tcp.local`.
    types: BTreeSet<String>,
    /// Who offers each service type: (type, instance name) and the address that said so.
    instances: BTreeMap<(String, String), Ipv4Addr>,
    /// Where an instance is: its port and host name. Keys are lowercase.
    services: HashMap<String, (u16, String)>,
    texts: HashMap<String, Vec<String>>,
    addresses: HashMap<String, Ipv4Addr>,
}

impl Directory {
    fn absorb(&mut self, packet: &[u8], from: Ipv4Addr) {
        for record in parse_records(packet) {
            let owner = record.name.to_lowercase();
            match record.data {
                Data::Pointer(target) if owner == SERVICE_LIST_NAME => {
                    if is_service_type(&target) && self.types.len() < MAX_TYPES {
                        self.types.insert(target);
                    }
                }
                Data::Pointer(target) if is_service_type(&owner) => {
                    if self.instances.len() < MAX_INSTANCES {
                        self.instances.entry((owner, target)).or_insert(from);
                    }
                }
                Data::Service { port, target } => {
                    self.services.insert(owner, (port, target.to_lowercase()));
                }
                Data::Text(entries) => {
                    self.texts.insert(owner, entries);
                }
                Data::Address(address) => {
                    self.addresses.insert(owner, address);
                }
                _ => {}
            }
        }
    }

    fn entries(&self) -> Vec<ServiceEntry> {
        self.instances
            .iter()
            .map(|((service_type, instance), from)| {
                let key = instance.to_lowercase();
                let service = self.services.get(&key);
                let address = service
                    .and_then(|(_, host)| self.addresses.get(host))
                    .copied()
                    .unwrap_or(*from);
                let suffix = format!(".{service_type}");
                let name = instance.strip_suffix(&suffix).unwrap_or(instance);
                ServiceEntry {
                    kind: service_type
                        .strip_suffix(LOCAL_SUFFIX)
                        .unwrap_or(service_type)
                        .to_owned(),
                    name: clean_text(name.as_bytes()).unwrap_or_default(),
                    address: Some(address),
                    port: service.map(|(port, _)| *port),
                    host: service.and_then(|(_, host)| {
                        let bare = host.strip_suffix(LOCAL_SUFFIX).unwrap_or(host);
                        clean_text(bare.as_bytes())
                    }),
                    details: self.texts.get(&key).cloned().unwrap_or_default(),
                }
            })
            .collect()
    }
}

/// `_ipp._tcp.local`, but not a subtype such as `_printer._sub._ipp._tcp.local`.
fn is_service_type(name: &str) -> bool {
    name.starts_with('_') && name.ends_with(LOCAL_SUFFIX) && !name.contains(SUBTYPE_MARKER)
}

/// Reads answers into `directory` until `wait` runs out.
async fn collect(socket: &UdpSocket, directory: &mut Directory, wait: Duration) {
    let deadline = Instant::now() + wait;
    let mut buffer = vec![0; RECEIVE_BUFFER];
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Ok(Ok((length, SocketAddr::V4(from)))) =
            timeout(left, socket.recv_from(&mut buffer)).await
        else {
            break;
        };
        directory.absorb(&buffer[..length], *from.ip());
    }
}

async fn ask_all(socket: &UdpSocket, names: impl IntoIterator<Item = String>) {
    let group = SocketAddrV4::new(MDNS_GROUP, MDNS_PORT);
    for name in names {
        for _ in 0..QUERY_ROUNDS {
            let _ = socket.send_to(&query_packet(&name), group).await;
        }
    }
}

async fn browse_mdns() -> Vec<ServiceEntry> {
    let Some(_permit) = socket_permit().await else {
        return Vec::new();
    };
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
        return Vec::new();
    };
    let mut directory = Directory::default();
    ask_all(&socket, [SERVICE_LIST_NAME.to_owned()]).await;
    collect(&socket, &mut directory, TYPE_LIST_WAIT).await;
    ask_all(&socket, directory.types.iter().cloned().collect::<Vec<_>>()).await;
    collect(&socket, &mut directory, INSTANCE_WAIT).await;
    directory.entries()
}

async fn browse_upnp() -> Vec<ServiceEntry> {
    let found = announcements(SSDP_WAIT).await;
    let mut names = JoinSet::new();
    let mut asked = BTreeSet::new();
    for announcement in &found {
        if let Some(location) = &announcement.location
            && asked.len() < MAX_UPNP_DESCRIPTIONS
            && asked.insert(announcement.ip)
        {
            let (ip, location) = (announcement.ip, location.clone());
            names.spawn(async move { (ip, friendly_name(location).await) });
        }
    }
    let mut friendly: HashMap<Ipv4Addr, String> = HashMap::new();
    while let Some(Ok((ip, Some(name)))) = names.join_next().await {
        friendly.insert(ip, name);
    }
    found
        .into_iter()
        .filter_map(|announcement| {
            let device = upnp_device_type(&announcement.search_target)?;
            Some(ServiceEntry {
                kind: format!("{UPNP_KIND_PREFIX}{device}"),
                name: friendly
                    .get(&announcement.ip)
                    .and_then(|name| clean_text(name.as_bytes()))
                    .unwrap_or_else(|| announcement.ip.to_string()),
                address: Some(announcement.ip),
                port: announcement.location.as_ref().map(|location| location.port),
                host: None,
                details: announcement
                    .location
                    .iter()
                    .map(|location| {
                        format!(
                            "http://{}:{}{}",
                            location.host, location.port, location.path
                        )
                    })
                    .collect(),
            })
        })
        .collect()
}

/// `MediaServer` from `urn:schemas-upnp-org:device:MediaServer:1`. Services and the
/// bookkeeping targets (`uuid:`, `upnp:rootdevice`) are not devices a person would look for.
fn upnp_device_type(search_target: &str) -> Option<&str> {
    let mut parts = search_target.strip_prefix("urn:")?.split(':');
    let _domain = parts.next()?;
    (parts.next()? == "device")
        .then(|| parts.next())
        .flatten()
        .filter(|device| !device.is_empty())
}

/// Everything found on the network, sorted by kind and then name.
pub async fn browse() -> Vec<ServiceEntry> {
    let (mdns, upnp) = tokio::join!(browse_mdns(), browse_upnp());
    let mut entries = mdns;
    entries.extend(upnp);
    entries.sort_by(|a, b| {
        (&a.kind, a.name.to_lowercase(), a.address).cmp(&(
            &b.kind,
            b.name.to_lowercase(),
            b.address,
        ))
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_name(name: &str) -> Vec<u8> {
        let mut bytes = Vec::new();
        for label in name.split('.') {
            bytes.push(label.len() as u8);
            bytes.extend(label.as_bytes());
        }
        bytes.push(0);
        bytes
    }

    fn record(name: &str, kind: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = encode_name(name);
        bytes.extend(kind.to_be_bytes());
        bytes.extend(0x8001u16.to_be_bytes());
        bytes.extend([0, 0, 0, 120]);
        bytes.extend((data.len() as u16).to_be_bytes());
        bytes.extend(data);
        bytes
    }

    fn response(answers: u16, additional: u16, records: &[Vec<u8>]) -> Vec<u8> {
        let mut packet = vec![0, 0, 0x84, 0, 0, 0];
        packet.extend(answers.to_be_bytes());
        packet.extend([0, 0]);
        packet.extend(additional.to_be_bytes());
        for record in records {
            packet.extend(record);
        }
        packet
    }

    fn srv(port: u16, target: &str) -> Vec<u8> {
        let mut data = vec![0, 0, 0, 0];
        data.extend(port.to_be_bytes());
        data.extend(encode_name(target));
        data
    }

    fn txt(entries: &[&str]) -> Vec<u8> {
        entries
            .iter()
            .flat_map(|entry| std::iter::once(entry.len() as u8).chain(entry.bytes()))
            .collect()
    }

    const PRINTER: &str = "Brother HL._ipp._tcp.local";

    fn printer_answer() -> Vec<u8> {
        response(
            1,
            3,
            &[
                record("_ipp._tcp.local", RECORD_PTR, &encode_name(PRINTER)),
                record(PRINTER, RECORD_SRV, &srv(631, "brother.local")),
                record(PRINTER, RECORD_TXT, &txt(&["rp=ipp/print", "ty=HL-L2350"])),
                record("brother.local", RECORD_A, &[192, 168, 1, 40]),
            ],
        )
    }

    #[test]
    fn a_printer_is_found_with_its_port_address_and_text() {
        let mut directory = Directory::default();
        directory.absorb(&printer_answer(), Ipv4Addr::new(192, 168, 1, 99));
        assert_eq!(
            directory.entries(),
            [ServiceEntry {
                kind: "_ipp._tcp".to_owned(),
                name: "Brother HL".to_owned(),
                address: Some(Ipv4Addr::new(192, 168, 1, 40)),
                port: Some(631),
                host: Some("brother".to_owned()),
                details: vec!["rp=ipp/print".to_owned(), "ty=HL-L2350".to_owned()],
            }]
        );
    }

    #[test]
    fn without_an_address_record_the_sender_is_the_address() {
        let packet = response(
            1,
            0,
            &[record("_ipp._tcp.local", RECORD_PTR, &encode_name(PRINTER))],
        );
        let mut directory = Directory::default();
        directory.absorb(&packet, Ipv4Addr::new(192, 168, 1, 99));
        let entries = directory.entries();
        assert_eq!(entries[0].address, Some(Ipv4Addr::new(192, 168, 1, 99)));
        assert_eq!(entries[0].port, None);
    }

    #[test]
    fn service_types_are_learned_from_the_list_and_subtypes_are_skipped() {
        let packet = response(
            3,
            0,
            &[
                record(
                    SERVICE_LIST_NAME,
                    RECORD_PTR,
                    &encode_name("_ipp._tcp.local"),
                ),
                record(
                    SERVICE_LIST_NAME,
                    RECORD_PTR,
                    &encode_name("_printer._sub._ipp._tcp.local"),
                ),
                record(
                    SERVICE_LIST_NAME,
                    RECORD_PTR,
                    &encode_name("not-a-service.local"),
                ),
            ],
        );
        let mut directory = Directory::default();
        directory.absorb(&packet, Ipv4Addr::LOCALHOST);
        assert_eq!(
            directory.types.iter().collect::<Vec<_>>(),
            ["_ipp._tcp.local"]
        );
    }

    #[test]
    fn names_are_matched_whatever_their_case() {
        let mut directory = Directory::default();
        let packet = response(
            1,
            2,
            &[
                record("_IPP._tcp.local", RECORD_PTR, &encode_name(PRINTER)),
                record(
                    "BROTHER HL._ipp._tcp.local",
                    RECORD_SRV,
                    &srv(631, "Brother.local"),
                ),
                record("brother.LOCAL", RECORD_A, &[10, 0, 0, 3]),
            ],
        );
        directory.absorb(&packet, Ipv4Addr::LOCALHOST);
        let entries = directory.entries();
        assert_eq!(entries[0].address, Some(Ipv4Addr::new(10, 0, 0, 3)));
        assert_eq!(entries[0].port, Some(631));
    }

    #[test]
    fn broken_packets_give_what_came_before_the_break() {
        let mut cut = printer_answer();
        cut.truncate(cut.len() - 5);
        let mut directory = Directory::default();
        directory.absorb(&cut, Ipv4Addr::LOCALHOST);
        assert_eq!(directory.entries().len(), 1);
        directory.absorb(&[], Ipv4Addr::LOCALHOST);
        directory.absorb(&[0; 12], Ipv4Addr::LOCALHOST);
        directory.absorb(
            &[0, 0, 0x84, 0, 0, 9, 0, 9, 0, 9, 0, 0, 200],
            Ipv4Addr::LOCALHOST,
        );
        assert_eq!(directory.entries().len(), 1);
    }

    #[test]
    fn text_records_are_cleaned_and_bounded() {
        assert_eq!(parse_text(&txt(&["a=1", "b=\u{7}2"])), ["a=1", "b= 2"]);
        assert_eq!(parse_text(&[9, b'x']), Vec::<String>::new());
        assert_eq!(
            parse_text(&[6, b'i', b'd', b'=', 0xFF, 0xFE, 0x01]),
            ["id=(binary)"]
        );
        assert_eq!(parse_text(&[3, 0xFF, 0xFE, 0x01]), Vec::<String>::new());
        let many: Vec<String> = (0..40).map(|i| format!("k{i}=v")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(parse_text(&txt(&refs)).len(), MAX_TEXT_ENTRIES);
    }

    #[test]
    fn the_number_of_things_remembered_is_capped() {
        let mut directory = Directory::default();
        for i in 0..MAX_INSTANCES + 50 {
            let instance = format!("dev{i}._x._tcp.local");
            let packet = response(
                1,
                0,
                &[record("_x._tcp.local", RECORD_PTR, &encode_name(&instance))],
            );
            directory.absorb(&packet, Ipv4Addr::LOCALHOST);
        }
        assert_eq!(directory.instances.len(), MAX_INSTANCES);
    }

    #[test]
    fn upnp_devices_are_named_and_services_are_left_out() {
        assert_eq!(
            upnp_device_type("urn:schemas-upnp-org:device:MediaServer:1"),
            Some("MediaServer")
        );
        assert_eq!(
            upnp_device_type("urn:vendor-com:device:Thing:2"),
            Some("Thing")
        );
        assert_eq!(
            upnp_device_type("urn:schemas-upnp-org:service:ContentDirectory:1"),
            None
        );
        assert_eq!(upnp_device_type("upnp:rootdevice"), None);
        assert_eq!(upnp_device_type("uuid:1234"), None);
    }

    #[test]
    fn kinds_get_plain_titles_and_unknown_ones_are_tidied() {
        assert_eq!(kind_title("_ipp._tcp"), "Printers");
        assert_eq!(kind_title("upnp:MediaServer"), "UPnP MediaServer");
        assert_eq!(kind_title("_weird-thing._udp"), "weird-thing (udp)");
        assert_eq!(kind_title("odd"), "odd");
    }

    fn entry(name: &str, host: Option<&str>) -> ServiceEntry {
        ServiceEntry {
            kind: "_x._tcp".to_owned(),
            name: name.to_owned(),
            address: Some(Ipv4Addr::new(10, 0, 0, 7)),
            port: None,
            host: host.map(str::to_owned),
            details: Vec::new(),
        }
    }

    #[test]
    fn names_made_of_hardware_addresses_and_identifiers_give_way_to_the_host() {
        assert_eq!(
            entry("9E74262E9FCB@Living room", None).display_name(),
            "Living room"
        );
        assert_eq!(
            entry("Brother HL", Some("brother")).display_name(),
            "Brother HL"
        );
        let paired = entry(
            "56:2c:f3:77:6d:46@fe80::542c:f3ff:fe77:6d46-supportsRP-26",
            Some("iphone"),
        );
        assert_eq!(paired.display_name(), "iphone");
        let id = "7AA58048-275C-4C7E-A1CE-562A160C235F";
        assert_eq!(entry(id, None).display_name(), "10.0.0.7");
        assert_eq!(entry("fed-cafe", None).display_name(), "fed-cafe");
    }

    #[test]
    fn only_short_texts_with_a_value_are_worth_reading() {
        let mut shown = entry("x", None);
        shown.details = vec![
            "model=Mac16,5".to_owned(),
            "at=".to_owned(),
            format!("pk={}", "a".repeat(64)),
            "flag".to_owned(),
            "vs=1".to_owned(),
        ];
        let (readable, hidden) = shown.readable_details();
        assert_eq!(readable, ["model=Mac16,5", "flag", "vs=1"]);
        assert_eq!(hidden, 2);
    }
}
