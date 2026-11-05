//! The port forwards a router has open, read over UPnP IGD.
//!
//! This is read-only: the only action ever sent is `GetGenericPortMappingEntry`, which asks
//! for one mapping by number. Nothing is ever added or removed. The router is found as the
//! default gateway, and only its own address is ever contacted.

use std::collections::HashSet;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::Duration;

use crate::gateway::default_gateway;
use crate::local_http::{HttpLocation, exchange, get, parse_location};
use crate::snmp::clean_text;
use crate::ssdp::{
    DeviceDescription, MULTICAST_GROUP, SSDP_PORT, collect_locations, element_text,
    parse_description,
};

const SEARCH: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: urn:schemas-upnp-org:device:InternetGatewayDevice:1\r\n\r\n";
const SEARCH_WAIT: Duration = Duration::from_millis(1500);
/// A router with thousands of forwards is not a home router; stop and say so.
const MAX_MAPPINGS: u32 = 256;
/// Longest text taken from a router for a description.
const MAX_TEXT_CHARS: usize = 120;
const OK: u16 = 200;
/// `SpecifiedArrayIndexInvalid`: there is no mapping with that number, so the list is over.
const ERROR_INDEX_INVALID: u32 = 713;
/// `NoSuchEntryInArray`: a number in the middle was removed; the list goes on.
const ERROR_NO_SUCH_ENTRY: u32 = 714;
const CONNECTION_SERVICES: [&str; 2] = ["WANIPConnection", "WANPPPConnection"];

/// One forward: traffic arriving at the router on `external_port` goes to `internal_client`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortMapping {
    /// `TCP` or `UDP`.
    pub protocol: String,
    pub external_port: u16,
    pub internal_client: String,
    pub internal_port: u16,
    pub description: String,
    pub enabled: bool,
    /// Seconds until the forward expires; 0 means it does not.
    pub lease_seconds: u32,
    /// Only this outside address may use the forward. Empty for any.
    pub remote_host: String,
}

impl PortMapping {
    /// The inside address, when the router gave one.
    pub fn internal_ip(&self) -> Option<Ipv4Addr> {
        self.internal_client.parse().ok()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpnpReport {
    pub gateway: Ipv4Addr,
    pub device: DeviceDescription,
    pub mappings: Vec<PortMapping>,
    /// The router has more forwards than were read.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpnpError {
    NoGateway,
    /// The router did not answer UPnP at all, which is what a router with UPnP off does.
    NoAnswer,
    /// The router answered, but offers no port forwarding service.
    NoPortService,
    /// The router would not list its forwards. Carries what it said.
    Refused(String),
    Failed,
}

impl fmt::Display for UpnpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoGateway => f.write_str("This computer has no default gateway to ask"),
            Self::NoAnswer => f.write_str(
                "The router does not answer UPnP. That usually means UPnP is turned off on it, \
                 so no program can open ports on it by itself.",
            ),
            Self::NoPortService => f.write_str(
                "The router answers UPnP but does not offer port forwarding, so there is nothing to list",
            ),
            Self::Refused(reason) => write!(f, "The router would not list its port forwards: {reason}"),
            Self::Failed => f.write_str("The router stopped answering while its forwards were read"),
        }
    }
}

/// The router's port forwards, read from the default gateway.
pub async fn read_port_mappings() -> Result<UpnpReport, UpnpError> {
    let gateway = tokio::task::spawn_blocking(default_gateway)
        .await
        .ok()
        .flatten()
        .ok_or(UpnpError::NoGateway)?;
    let location = find_description(gateway).await.ok_or(UpnpError::NoAnswer)?;
    read_from(gateway, &location).await
}

/// Asks the gateway directly, then the whole network, for where its description is.
async fn find_description(gateway: Ipv4Addr) -> Option<HttpLocation> {
    let targets = HashSet::from([gateway]);
    for address in [
        SocketAddrV4::new(gateway, SSDP_PORT),
        SocketAddrV4::new(MULTICAST_GROUP, SSDP_PORT),
    ] {
        let mut found = collect_locations(SEARCH, address, &targets, SEARCH_WAIT).await;
        if let Some(location) = found.remove(&gateway) {
            return Some(location);
        }
    }
    None
}

async fn read_from(gateway: Ipv4Addr, location: &HttpLocation) -> Result<UpnpReport, UpnpError> {
    let description = get(location).await.ok_or(UpnpError::NoAnswer)?;
    let service = port_service(&description.body, location).ok_or(UpnpError::NoPortService)?;
    let mut mappings = Vec::new();
    let mut truncated = false;
    for index in 0..MAX_MAPPINGS {
        match read_mapping(&service, index).await? {
            Entry::Mapping(mapping) => mappings.push(mapping),
            Entry::Skip => {}
            Entry::End => break,
        }
        truncated = index + 1 == MAX_MAPPINGS;
    }
    Ok(UpnpReport {
        gateway,
        device: parse_description(&description.body),
        mappings,
        truncated,
    })
}

/// Where and how to ask the router about forwards.
struct PortService {
    control: HttpLocation,
    /// The service type, such as `urn:schemas-upnp-org:service:WANIPConnection:1`.
    kind: String,
}

/// The connection service in a device description, with its control address. A control
/// address on any other host than the one that gave the description is not used.
fn port_service(description: &str, described_at: &HttpLocation) -> Option<PortService> {
    description.split("<service>").skip(1).find_map(|block| {
        let kind = element_text(block, "serviceType")?;
        CONNECTION_SERVICES
            .iter()
            .any(|name| kind.contains(name))
            .then_some(())?;
        let control = control_location(&element_text(block, "controlURL")?, described_at)?;
        Some(PortService { control, kind })
    })
}

fn control_location(url: &str, described_at: &HttpLocation) -> Option<HttpLocation> {
    if url.starts_with("http://") {
        return parse_location(url).filter(|location| location.host == described_at.host);
    }
    let path = if url.starts_with('/') {
        url.to_owned()
    } else {
        format!("/{url}")
    };
    Some(HttpLocation {
        path,
        ..described_at.clone()
    })
}

enum Entry {
    Mapping(PortMapping),
    /// A number whose forward was removed; the next one may exist.
    Skip,
    End,
}

async fn read_mapping(service: &PortService, index: u32) -> Result<Entry, UpnpError> {
    let body = format!(
        "<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" \
         s:encodingStyle=\"http://schemas.xmlsoap.org/soap/encoding/\"><s:Body>\
         <u:GetGenericPortMappingEntry xmlns:u=\"{kind}\">\
         <NewPortMappingIndex>{index}</NewPortMappingIndex>\
         </u:GetGenericPortMappingEntry></s:Body></s:Envelope>",
        kind = service.kind
    );
    let control = &service.control;
    let request = format!(
        "POST {path} HTTP/1.0\r\nHost: {host}:{port}\r\n\
         Content-Type: text/xml; charset=\"utf-8\"\r\n\
         SOAPAction: \"{kind}#GetGenericPortMappingEntry\"\r\n\
         Content-Length: {length}\r\nConnection: close\r\n\r\n{body}",
        path = control.path,
        host = control.host,
        port = control.port,
        kind = service.kind,
        length = body.len(),
    );
    let response = exchange(control, request).await.ok_or(UpnpError::Failed)?;
    if response.status == OK {
        return parse_mapping(&response.body)
            .map(Entry::Mapping)
            .ok_or(UpnpError::Failed);
    }
    let code = element_text(&response.body, "errorCode").and_then(|code| code.parse().ok());
    match code {
        Some(ERROR_INDEX_INVALID) => Ok(Entry::End),
        Some(ERROR_NO_SUCH_ENTRY) => Ok(Entry::Skip),
        // Some routers answer a bare fault at the end of the list instead of 713.
        _ if index > 0 => Ok(Entry::End),
        _ => Err(UpnpError::Refused(fault_text(
            &response.body,
            response.status,
        ))),
    }
}

fn fault_text(body: &str, status: u16) -> String {
    match (
        element_text(body, "errorCode"),
        element_text(body, "errorDescription"),
    ) {
        (Some(code), Some(description)) => format!("{description} ({code})"),
        (Some(code), None) => format!("error {code}"),
        _ => format!("HTTP {status}"),
    }
}

fn parse_mapping(xml: &str) -> Option<PortMapping> {
    let text = |tag| element_text(xml, tag);
    let cleaned = |tag| {
        text(tag)
            .and_then(|value| clean_text(value.as_bytes()))
            .map(|value| value.chars().take(MAX_TEXT_CHARS).collect::<String>())
            .unwrap_or_default()
    };
    Some(PortMapping {
        protocol: text("NewProtocol")?.to_uppercase(),
        external_port: text("NewExternalPort")?.parse().ok()?,
        internal_client: cleaned("NewInternalClient"),
        internal_port: text("NewInternalPort")?.parse().ok()?,
        description: cleaned("NewPortMappingDescription"),
        enabled: text("NewEnabled").is_none_or(|value| value != "0"),
        lease_seconds: text("NewLeaseDuration")
            .and_then(|value| value.parse().ok())
            .unwrap_or(0),
        remote_host: cleaned("NewRemoteHost"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    type Log = Arc<Mutex<Vec<String>>>;

    const DESCRIPTION: &str = "<root><device><friendlyName>Home Router</friendlyName>\
        <deviceList><device><deviceList><device><serviceList>\
        <service><serviceType>urn:schemas-upnp-org:service:WANCommonInterfaceConfig:1</serviceType>\
        <controlURL>/ctl/common</controlURL></service>\
        <service><serviceType>urn:schemas-upnp-org:service:WANIPConnection:1</serviceType>\
        <controlURL>/ctl/ip</controlURL></service>\
        </serviceList></device></deviceList></device></deviceList></device></root>";

    fn entry(external: u16, client: &str, description: &str) -> String {
        format!(
            "<s:Envelope><s:Body><u:GetGenericPortMappingEntryResponse>\
             <NewRemoteHost></NewRemoteHost><NewExternalPort>{external}</NewExternalPort>\
             <NewProtocol>tcp</NewProtocol><NewInternalPort>22</NewInternalPort>\
             <NewInternalClient>{client}</NewInternalClient><NewEnabled>1</NewEnabled>\
             <NewPortMappingDescription>{description}</NewPortMappingDescription>\
             <NewLeaseDuration>3600</NewLeaseDuration>\
             </u:GetGenericPortMappingEntryResponse></s:Body></s:Envelope>"
        )
    }

    fn fault(code: u32) -> String {
        format!(
            "<s:Envelope><s:Body><s:Fault><detail><UPnPError><errorCode>{code}</errorCode>\
             <errorDescription>Invalid Index</errorDescription></UPnPError></detail>\
             </s:Fault></s:Body></s:Envelope>"
        )
    }

    /// A router that serves its description and answers each numbered request from `answers`.
    /// Anything past the list gets fault 713.
    async fn fake_router(answers: Vec<String>) -> (HttpLocation, Log, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Log::default();
        let seen = Arc::clone(&log);
        let handle = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut buffer = vec![0; 4096];
                let length = stream.read(&mut buffer).await.unwrap_or(0);
                let request = String::from_utf8_lossy(&buffer[..length]).into_owned();
                let (status, body) = if request.starts_with("GET") {
                    (200, DESCRIPTION.to_owned())
                } else {
                    let index: usize = request
                        .split("<NewPortMappingIndex>")
                        .nth(1)
                        .and_then(|rest| rest.split('<').next())
                        .and_then(|number| number.parse().ok())
                        .unwrap();
                    match answers.get(index) {
                        Some(answer) if answer.contains("errorCode") => (500, answer.clone()),
                        Some(answer) => (200, answer.clone()),
                        None => (500, fault(713)),
                    }
                };
                seen.lock().unwrap().push(request);
                let reply = format!("HTTP/1.0 {status} X\r\n\r\n{body}");
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        let location = HttpLocation {
            host: Ipv4Addr::LOCALHOST,
            port,
            path: "/desc.xml".to_owned(),
        };
        (location, log, handle)
    }

    #[test]
    fn the_connection_service_is_found_among_the_others() {
        let at = HttpLocation {
            host: Ipv4Addr::new(192, 168, 1, 1),
            port: 5000,
            path: "/desc.xml".to_owned(),
        };
        let service = port_service(DESCRIPTION, &at).unwrap();
        assert_eq!(
            service.kind,
            "urn:schemas-upnp-org:service:WANIPConnection:1"
        );
        assert_eq!(service.control.path, "/ctl/ip");
        assert_eq!(
            (service.control.host, service.control.port),
            (at.host, 5000)
        );
        assert!(port_service("<root></root>", &at).is_none());
    }

    #[test]
    fn a_control_address_on_another_host_is_not_used() {
        let at = HttpLocation {
            host: Ipv4Addr::new(192, 168, 1, 1),
            port: 5000,
            path: "/d.xml".to_owned(),
        };
        assert!(control_location("http://10.0.0.9/ctl", &at).is_none());
        assert_eq!(
            control_location("http://192.168.1.1:5001/ctl", &at)
                .unwrap()
                .port,
            5001
        );
        assert_eq!(control_location("ctl/ip", &at).unwrap().path, "/ctl/ip");
    }

    #[test]
    fn a_forward_is_read_with_its_lease_and_cleaned_text() {
        let mapping = parse_mapping(&entry(2222, "192.168.1.20", "ssh &amp; \u{7}bell")).unwrap();
        assert_eq!(mapping.protocol, "TCP");
        assert_eq!((mapping.external_port, mapping.internal_port), (2222, 22));
        assert_eq!(mapping.internal_ip(), Some(Ipv4Addr::new(192, 168, 1, 20)));
        assert_eq!(mapping.description, "ssh &  bell");
        assert_eq!(mapping.lease_seconds, 3600);
        assert!(mapping.enabled);
        assert_eq!(parse_mapping("<x/>"), None);
        assert_eq!(
            parse_mapping(&entry(2222, "nas", ""))
                .unwrap()
                .internal_ip(),
            None
        );
    }

    #[tokio::test]
    async fn every_forward_is_listed_until_the_router_runs_out() {
        let answers = vec![
            entry(80, "192.168.1.5", "web"),
            entry(443, "192.168.1.5", "tls"),
        ];
        let (location, _log, server) = fake_router(answers).await;
        let report = read_from(Ipv4Addr::LOCALHOST, &location).await.unwrap();
        server.abort();
        let ports: Vec<u16> = report.mappings.iter().map(|m| m.external_port).collect();
        assert_eq!(ports, [80, 443]);
        assert_eq!(report.device.friendly_name.as_deref(), Some("Home Router"));
        assert!(!report.truncated);
    }

    #[tokio::test]
    async fn a_removed_entry_in_the_middle_is_skipped() {
        let answers = vec![
            entry(80, "192.168.1.5", "web"),
            fault(714),
            entry(8080, "192.168.1.6", "alt"),
        ];
        let (location, _log, server) = fake_router(answers).await;
        let report = read_from(Ipv4Addr::LOCALHOST, &location).await.unwrap();
        server.abort();
        let ports: Vec<u16> = report.mappings.iter().map(|m| m.external_port).collect();
        assert_eq!(ports, [80, 8080]);
    }

    #[tokio::test]
    async fn a_router_with_no_forwards_gives_an_empty_list_not_an_error() {
        let (location, _log, server) = fake_router(Vec::new()).await;
        let report = read_from(Ipv4Addr::LOCALHOST, &location).await.unwrap();
        server.abort();
        assert!(report.mappings.is_empty());
    }

    #[tokio::test]
    async fn a_router_that_refuses_the_first_request_says_why() {
        let (location, _log, server) = fake_router(vec![fault(606)]).await;
        let error = read_from(Ipv4Addr::LOCALHOST, &location).await.unwrap_err();
        server.abort();
        assert_eq!(error, UpnpError::Refused("Invalid Index (606)".to_owned()));
    }

    #[tokio::test]
    async fn only_the_list_action_is_ever_sent() {
        let (location, log, server) = fake_router(vec![entry(80, "192.168.1.5", "web")]).await;
        read_from(Ipv4Addr::LOCALHOST, &location).await.unwrap();
        server.abort();
        let requests = log.lock().unwrap();
        let posts: Vec<&String> = requests.iter().filter(|r| r.starts_with("POST")).collect();
        assert_eq!(posts.len(), 2, "one forward, then the end of the list");
        for post in posts {
            assert!(post.contains("#GetGenericPortMappingEntry\""), "{post}");
            assert!(!post.contains("AddPortMapping") && !post.contains("DeletePortMapping"));
        }
    }

    #[test]
    fn errors_say_what_to_do() {
        assert!(UpnpError::NoAnswer.to_string().contains("turned off"));
        assert!(UpnpError::NoGateway.to_string().contains("gateway"));
    }
}
