//! A small HTTP/1.0 client for devices on the local network, such as a router.
//!
//! One request, one answer, then the connection closes. Only plain `http://` and only an IPv4
//! address as the host, since a router or a media server is never reached by name.

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

use crate::socket_budget::socket_permit;

/// Answers from these devices are small; anything larger is cut off here.
const MAX_RESPONSE_BYTES: u64 = 64 * 1024;
const EXCHANGE_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_PORT: u16 = 80;
const HEADER_END: &[u8] = b"\r\n\r\n";

/// A plain `http://` URL split up. Other schemes are not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HttpLocation {
    pub host: Ipv4Addr,
    pub port: u16,
    pub path: String,
}

pub(crate) fn parse_location(url: &str) -> Option<HttpLocation> {
    let rest = url.trim().strip_prefix("http://")?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let default_port = DEFAULT_PORT.to_string();
    let (host, port) = authority
        .split_once(':')
        .unwrap_or((authority, default_port.as_str()));
    Some(HttpLocation {
        host: host.parse().ok()?,
        port: port.parse().ok()?,
        path: format!("/{path}"),
    })
}

/// What a device answered: the status code and the text after the headers.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Response {
    pub status: u16,
    pub body: String,
}

fn parse_response(bytes: &[u8]) -> Option<Response> {
    let text = String::from_utf8_lossy(bytes);
    let status = text
        .lines()
        .next()?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    let body_start = bytes
        .windows(HEADER_END.len())
        .position(|window| window == HEADER_END)
        .map_or(bytes.len(), |at| at + HEADER_END.len());
    Some(Response {
        status,
        body: String::from_utf8_lossy(&bytes[body_start..]).into_owned(),
    })
}

/// Sends `request`, which is the whole text of an HTTP/1.0 request, and reads the answer.
pub(crate) async fn exchange(location: &HttpLocation, request: String) -> Option<Response> {
    let work = async {
        let _permit = socket_permit().await?;
        let mut stream = TcpStream::connect((location.host, location.port))
            .await
            .ok()?;
        stream.write_all(request.as_bytes()).await.ok()?;
        let mut answer = Vec::new();
        stream
            .take(MAX_RESPONSE_BYTES)
            .read_to_end(&mut answer)
            .await
            .ok()?;
        parse_response(&answer)
    };
    timeout(EXCHANGE_TIMEOUT, work).await.ok()?
}

/// `GET`s the page at `location`.
pub(crate) async fn get(location: &HttpLocation) -> Option<Response> {
    exchange(
        location,
        format!(
            "GET {} HTTP/1.0\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
            location.path, location.host, location.port
        ),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

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
    fn splits_the_status_from_the_body() {
        let answer = b"HTTP/1.0 500 Internal Server Error\r\nA: b\r\n\r\n<x>1</x>";
        assert_eq!(
            parse_response(answer),
            Some(Response {
                status: 500,
                body: "<x>1</x>".to_owned()
            })
        );
        assert_eq!(parse_response(b"nonsense"), None);
        assert_eq!(parse_response(b"HTTP/1.0 200 OK").unwrap().body, "");
    }

    #[tokio::test]
    async fn gets_a_page_over_http() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 256];
            let _ = stream.read(&mut request).await;
            let _ = stream.write_all(b"HTTP/1.0 200 OK\r\n\r\nhello").await;
        });
        let location = HttpLocation {
            host: Ipv4Addr::LOCALHOST,
            port,
            path: "/d.xml".to_owned(),
        };
        let response = get(&location).await.unwrap();
        assert_eq!((response.status, response.body.as_str()), (200, "hello"));
    }
}
