//! The checks that need the internet: the public address, whether IPv6 works, and whether a
//! sign-in page is in the way.
//!
//! Each one makes a single request to Cloudflare, and nothing is sent unless it is asked for.
//! [`CONTACTS`] lists what is contacted, so a person can be told before they press the button.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::https::get_response;
use crate::socket_budget::socket_permit;
use crate::ssdp::header_value;

/// What the checks contact.
pub const CONTACTS: &str = "Cloudflare: 1.1.1.1 (public address), cp.cloudflare.com (sign-in page check) and 2606:4700:4700::1111 (IPv6)";

const TRACE_HOST: &str = "1.1.1.1";
const TRACE_PATH: &str = "/cdn-cgi/trace";
const PORTAL_HOST: &str = "cp.cloudflare.com";
const PORTAL_PATH: &str = "/generate_204";
const HTTP_PORT: u16 = 80;
/// A host that only has an IPv6 address, so reaching it proves IPv6 works.
const IPV6_HOST: &str = "[2606:4700:4700::1111]:443";
const IPV6_WAIT: Duration = Duration::from_secs(4);
const PORTAL_WAIT: Duration = Duration::from_secs(5);
const HEAD_BYTES: u64 = 4096;
const STATUS_OK: u16 = 200;
const STATUS_NO_CONTENT: u16 = 204;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ipv6Status {
    Works,
    Fails(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortalStatus {
    /// The internet answered as it should: no sign-in page is in the way.
    Open,
    /// Something answered in place of the internet, such as a hotel sign-in page.
    SignInNeeded {
        location: Option<String>,
    },
    Unknown(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnlineFacts {
    pub public_address: Result<IpAddr, String>,
    pub ipv6: Ipv6Status,
    pub captive_portal: PortalStatus,
}

/// Runs the three checks together.
pub async fn check_online() -> OnlineFacts {
    let (public_address, ipv6, captive_portal) =
        tokio::join!(public_address(), ipv6_works(), captive_portal());
    OnlineFacts {
        public_address,
        ipv6,
        captive_portal,
    }
}

async fn public_address() -> Result<IpAddr, String> {
    let response = get_response(TRACE_HOST, TRACE_PATH).await?;
    if response.status != STATUS_OK {
        return Err(format!(
            "The service answered with status {}",
            response.status
        ));
    }
    parse_trace_address(&String::from_utf8_lossy(&response.body))
        .ok_or_else(|| "The service did not say which address it saw".to_owned())
}

/// The `ip=` line of the Cloudflare trace page.
fn parse_trace_address(text: &str) -> Option<IpAddr> {
    text.lines()
        .find_map(|line| line.strip_prefix("ip="))
        .and_then(|address| address.trim().parse().ok())
}

async fn ipv6_works() -> Ipv6Status {
    let Some(_permit) = socket_permit().await else {
        return Ipv6Status::Fails("Too many connections are open".to_owned());
    };
    let address: SocketAddr = IPV6_HOST.parse().expect("a fixed address");
    match timeout(IPV6_WAIT, TcpStream::connect(address)).await {
        Ok(Ok(_)) => Ipv6Status::Works,
        Ok(Err(error)) => Ipv6Status::Fails(error.to_string()),
        Err(_) => Ipv6Status::Fails("No answer".to_owned()),
    }
}

async fn captive_portal() -> PortalStatus {
    match timeout(PORTAL_WAIT, fetch_head(PORTAL_HOST, PORTAL_PATH)).await {
        Ok(Ok(head)) => portal_status(&head),
        Ok(Err(error)) => PortalStatus::Unknown(error),
        Err(_) => PortalStatus::Unknown("No answer".to_owned()),
    }
}

/// The start of the answer to a plain `http://` GET: the status line and the headers.
async fn fetch_head(host: &str, path: &str) -> Result<String, String> {
    let _permit = socket_permit().await;
    let mut stream = TcpStream::connect((host, HTTP_PORT))
        .await
        .map_err(|error| error.to_string())?;
    let request = format!(
        "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: lanping\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| error.to_string())?;
    let mut head = Vec::new();
    stream
        .take(HEAD_BYTES)
        .read_to_end(&mut head)
        .await
        .map_err(|error| error.to_string())?;
    Ok(String::from_utf8_lossy(&head).into_owned())
}

/// What the answer to the check page says. The page is meant to answer `204 No Content`; a
/// sign-in page in the way answers with a redirect or with a page of its own.
fn portal_status(head: &str) -> PortalStatus {
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok());
    match status {
        Some(STATUS_NO_CONTENT) => PortalStatus::Open,
        Some(300..=399) => PortalStatus::SignInNeeded {
            location: header_value(head, "location").map(str::to_owned),
        },
        Some(STATUS_OK) => PortalStatus::SignInNeeded { location: None },
        Some(other) => {
            PortalStatus::Unknown(format!("The check page answered with status {other}"))
        }
        None => PortalStatus::Unknown("The check page sent an unreadable answer".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_address_is_read_from_the_trace_page() {
        let page = "fl=1\nh=1.1.1.1\nip=203.0.113.9\nts=1\nloc=EE\n";
        assert_eq!(
            parse_trace_address(page),
            Some("203.0.113.9".parse().unwrap())
        );
        assert_eq!(
            parse_trace_address("ip=2001:db8::5\n"),
            Some("2001:db8::5".parse().unwrap())
        );
        assert_eq!(parse_trace_address("ip=nonsense\n"), None);
        assert_eq!(parse_trace_address(""), None);
    }

    #[test]
    fn a_204_means_no_sign_in_page_is_in_the_way() {
        assert_eq!(
            portal_status("HTTP/1.1 204 No Content\r\n\r\n"),
            PortalStatus::Open
        );
    }

    #[test]
    fn a_redirect_or_a_page_means_signing_in_is_needed() {
        assert_eq!(
            portal_status("HTTP/1.1 302 Found\r\nLocation: http://login.hotel.example/\r\n\r\n"),
            PortalStatus::SignInNeeded {
                location: Some("http://login.hotel.example/".to_owned())
            }
        );
        assert_eq!(
            portal_status("HTTP/1.0 200 OK\r\n\r\n<html>Welcome</html>"),
            PortalStatus::SignInNeeded { location: None }
        );
    }

    #[test]
    fn other_answers_are_not_taken_for_a_sign_in_page() {
        assert!(matches!(
            portal_status("HTTP/1.1 503 Service Unavailable\r\n\r\n"),
            PortalStatus::Unknown(text) if text.contains("503")
        ));
        assert!(matches!(portal_status(""), PortalStatus::Unknown(_)));
    }
}
