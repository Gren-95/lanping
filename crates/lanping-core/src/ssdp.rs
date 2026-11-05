//! SSDP/UPnP: asks the network who is there, then reads each answering
//! device's description for its name and model.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::local_http::{HttpLocation, get, parse_location};

pub(crate) const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
pub(crate) const SSDP_PORT: u16 = 1900;
const SEARCH_ATTEMPTS: usize = 2;
const SEARCH: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: ssdp:all\r\n\r\n";

/// What a device's UPnP description says about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceDescription {
    pub friendly_name: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

/// The `LOCATION` header of an SSDP response.
pub(crate) fn location_header(response: &str) -> Option<&str> {
    response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("location").then(|| value.trim())
    })
}

/// The text of the first `<tag>` element. This is a plain text search on purpose:
/// no XML parser means no entity or external reference handling on untrusted input.
pub(crate) fn element_text(xml: &str, tag: &str) -> Option<String> {
    let (_, after_open) = xml.split_once(&format!("<{tag}>"))?;
    let (text, _) = after_open.split_once(&format!("</{tag}>"))?;
    let text = decode_entities(text.trim());
    (!text.is_empty()).then_some(text)
}

/// Turns the five predefined XML entities back into characters. `&amp;` comes last, so
/// `&amp;lt;` stays the text `&lt;`.
fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

pub(crate) fn parse_description(xml: &str) -> DeviceDescription {
    DeviceDescription {
        friendly_name: element_text(xml, "friendlyName"),
        manufacturer: element_text(xml, "manufacturer"),
        model: element_text(xml, "modelName"),
    }
}

async fn read_description(location: HttpLocation) -> Option<DeviceDescription> {
    let response = get(&location).await?;
    Some(parse_description(&response.body))
}

/// Collects `LOCATION` URLs by answering address until `wait` runs out.
/// Answers from outside `targets`, and URLs on another host, are ignored.
pub(crate) async fn collect_locations(
    search: &str,
    search_address: SocketAddrV4,
    targets: &HashSet<Ipv4Addr>,
    wait: Duration,
) -> HashMap<Ipv4Addr, HttpLocation> {
    let mut found = HashMap::new();
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).await else {
        return found;
    };
    // UDP over Wi-Fi often loses multicast, so the search goes out more than once.
    for _ in 0..SEARCH_ATTEMPTS {
        if socket
            .send_to(search.as_bytes(), search_address)
            .await
            .is_err()
        {
            return found;
        }
    }
    let deadline = Instant::now() + wait;
    let mut buffer = [0; 2048];
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let Ok(Ok((length, SocketAddr::V4(from)))) =
            timeout(left, socket.recv_from(&mut buffer)).await
        else {
            break;
        };
        let text = String::from_utf8_lossy(&buffer[..length]);
        let ip = *from.ip();
        if targets.contains(&ip)
            && let Some(location) = location_header(&text).and_then(parse_location)
            && location.host == ip
        {
            found.entry(ip).or_insert(location);
        }
    }
    found
}

/// Asks the network for UPnP devices among `targets` and reads their descriptions.
pub async fn survey(targets: &[Ipv4Addr], wait: Duration) -> HashMap<Ipv4Addr, DeviceDescription> {
    let targets: HashSet<Ipv4Addr> = targets.iter().copied().collect();
    let mut reads = JoinSet::new();
    let group = SocketAddrV4::new(MULTICAST_GROUP, SSDP_PORT);
    for (ip, location) in collect_locations(SEARCH, group, &targets, wait).await {
        reads.spawn(async move { Some((ip, read_description(location).await?)) });
    }
    let mut descriptions = HashMap::new();
    while let Some(joined) = reads.join_next().await {
        if let Ok(Some((ip, description))) = joined {
            descriptions.insert(ip, description);
        }
    }
    descriptions
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_location_header_in_any_case() {
        let response =
            "HTTP/1.1 200 OK\r\nlocation: http://192.168.1.5:49152/desc.xml\r\nST: x\r\n";
        assert_eq!(
            location_header(response),
            Some("http://192.168.1.5:49152/desc.xml")
        );
        assert_eq!(location_header("HTTP/1.1 200 OK\r\n"), None);
    }

    #[test]
    fn xml_entities_are_decoded_once() {
        assert_eq!(
            element_text("<a>x &lt;y&gt; &amp;lt; &quot;z&quot;</a>", "a").as_deref(),
            Some("x <y> &lt; \"z\"")
        );
    }

    #[test]
    fn pulls_fields_from_a_description() {
        let xml = "<root><device><friendlyName> Living Room </friendlyName>\
            <manufacturer>Tom &amp; Co</manufacturer><modelName>TV-1</modelName></device></root>";
        assert_eq!(
            parse_description(xml),
            DeviceDescription {
                friendly_name: Some("Living Room".to_owned()),
                manufacturer: Some("Tom & Co".to_owned()),
                model: Some("TV-1".to_owned()),
            }
        );
        assert_eq!(parse_description("not xml"), DeviceDescription::default());
    }

    #[tokio::test]
    async fn collects_answers_from_targets_only_and_same_host_urls_only() {
        let responder = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let SocketAddr::V4(address) = responder.local_addr().unwrap() else {
            panic!("bound to IPv4");
        };
        tokio::spawn(async move {
            let mut buffer = [0; 512];
            let (_, from) = responder.recv_from(&mut buffer).await.unwrap();
            let elsewhere = b"HTTP/1.1 200 OK\r\nLOCATION: http://10.9.9.9/d.xml\r\n\r\n";
            responder.send_to(elsewhere, from).await.unwrap();
            let here = b"HTTP/1.1 200 OK\r\nLOCATION: http://127.0.0.1:8200/d.xml\r\n\r\n";
            responder.send_to(here, from).await.unwrap();
        });
        let targets = HashSet::from([Ipv4Addr::LOCALHOST]);
        let found = collect_locations(SEARCH, address, &targets, Duration::from_millis(300)).await;
        assert_eq!(found[&Ipv4Addr::LOCALHOST].port, 8200);

        let none =
            collect_locations(SEARCH, address, &HashSet::new(), Duration::from_millis(50)).await;
        assert!(none.is_empty());
    }
}
