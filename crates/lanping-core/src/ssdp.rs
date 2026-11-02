//! SSDP/UPnP: asks the network who is there, then reads each answering
//! device's description for its name and model.

use std::collections::{HashMap, HashSet};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::task::JoinSet;
use tokio::time::timeout;

use crate::socket_budget::socket_permit;

const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);
const SSDP_PORT: u16 = 1900;
const SEARCH_ATTEMPTS: usize = 2;
const SEARCH: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: ssdp:all\r\n\r\n";
/// Device descriptions are small; anything larger is cut off here.
const MAX_DESCRIPTION_BYTES: u64 = 64 * 1024;
const DESCRIPTION_TIMEOUT: Duration = Duration::from_secs(2);

/// What a device's UPnP description says about it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceDescription {
    pub friendly_name: Option<String>,
    pub manufacturer: Option<String>,
    pub model: Option<String>,
}

/// A plain `http://` URL split up. Other schemes are not read.
#[derive(Debug, PartialEq, Eq)]
struct HttpLocation {
    host: Ipv4Addr,
    port: u16,
    path: String,
}

fn parse_location(url: &str) -> Option<HttpLocation> {
    let rest = url.trim().strip_prefix("http://")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority.split_once(':').unwrap_or((authority, "80"));
    Some(HttpLocation {
        host: host.parse().ok()?,
        port: port.parse().ok()?,
        path: format!("/{path}"),
    })
}

/// The `LOCATION` header of an SSDP response.
fn location_header(response: &str) -> Option<&str> {
    response.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("location").then(|| value.trim())
    })
}

/// The text of the first `<tag>` element. This is a plain text search on purpose:
/// no XML parser means no entity or external reference handling on untrusted input.
fn element_text(xml: &str, tag: &str) -> Option<String> {
    let (_, after_open) = xml.split_once(&format!("<{tag}>"))?;
    let (text, _) = after_open.split_once(&format!("</{tag}>"))?;
    let text = text.trim().replace("&amp;", "&");
    (!text.is_empty()).then_some(text)
}

fn parse_description(xml: &str) -> DeviceDescription {
    DeviceDescription {
        friendly_name: element_text(xml, "friendlyName"),
        manufacturer: element_text(xml, "manufacturer"),
        model: element_text(xml, "modelName"),
    }
}

async fn read_description(location: HttpLocation) -> Option<DeviceDescription> {
    let exchange = async {
        let _permit = socket_permit().await?;
        let mut stream = TcpStream::connect((location.host, location.port))
            .await
            .ok()?;
        let request = format!(
            "GET {} HTTP/1.0\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            location.path, location.host, location.port
        );
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut body = Vec::new();
        stream
            .take(MAX_DESCRIPTION_BYTES)
            .read_to_end(&mut body)
            .await
            .ok()?;
        Some(parse_description(&String::from_utf8_lossy(&body)))
    };
    timeout(DESCRIPTION_TIMEOUT, exchange).await.ok()?
}

/// Collects `LOCATION` URLs by answering address until `wait` runs out.
/// Answers from outside `targets`, and URLs on another host, are ignored.
async fn collect_locations(
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
            .send_to(SEARCH.as_bytes(), search_address)
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
    for (ip, location) in collect_locations(group, &targets, wait).await {
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
    use tokio::net::TcpListener;

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
    fn accepts_only_plain_http_with_an_ip_host() {
        assert_eq!(
            parse_location("http://192.168.1.5:8080/a/b.xml"),
            Some(HttpLocation {
                host: Ipv4Addr::new(192, 168, 1, 5),
                port: 8080,
                path: "/a/b.xml".to_owned(),
            })
        );
        assert_eq!(parse_location("http://192.168.1.5").unwrap().port, 80);
        assert_eq!(parse_location("https://192.168.1.5/a"), None);
        assert_eq!(parse_location("http://example.com/a"), None);
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
        let found = collect_locations(address, &targets, Duration::from_millis(300)).await;
        assert_eq!(found[&Ipv4Addr::LOCALHOST].port, 8200);

        let none = collect_locations(address, &HashSet::new(), Duration::from_millis(50)).await;
        assert!(none.is_empty());
    }

    #[tokio::test]
    async fn reads_a_description_over_http() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 256];
            let _ = stream.read(&mut request).await;
            let _ = stream
                .write_all(b"HTTP/1.0 200 OK\r\n\r\n<modelName>M1</modelName>")
                .await;
        });
        let description = read_description(HttpLocation {
            host: Ipv4Addr::LOCALHOST,
            port,
            path: "/d.xml".to_owned(),
        })
        .await
        .unwrap();
        assert_eq!(description.model.as_deref(), Some("M1"));
    }
}
