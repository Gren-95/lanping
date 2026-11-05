//! Small HTTPS client pieces shared by the speed tests: a verified TLS connection, and a
//! plain GET that returns the body.

use std::io;
use std::ops::{Deref, DerefMut};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use rustls::ClientConfig;
use rustls::pki_types::ServerName;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::SemaphorePermit;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::socket_budget::socket_permit;
use crate::tls::system_roots;

pub(crate) const HTTPS_PORT: u16 = 443;
const CONNECT_WAIT: Duration = Duration::from_secs(8);
const GET_WAIT: Duration = Duration::from_secs(15);
const MAX_GET_BYTES: u64 = 4 * 1024 * 1024;

pub(crate) type TlsStream = tokio_rustls::client::TlsStream<TcpStream>;

/// A TLS connection that holds its place in the socket budget for as long as it lives.
pub(crate) struct Connection {
    stream: TlsStream,
    _permit: Option<SemaphorePermit<'static>>,
}

impl Deref for Connection {
    type Target = TlsStream;

    fn deref(&self) -> &TlsStream {
        &self.stream
    }
}

impl DerefMut for Connection {
    fn deref_mut(&mut self) -> &mut TlsStream {
        &mut self.stream
    }
}

impl AsyncRead for Connection {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for Connection {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(context, bytes)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

/// A TLS client that trusts the system's root certificates.
pub(crate) async fn connector() -> Result<TlsConnector, String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| format!("Could not set up TLS: {error}"))?
        .with_root_certificates(system_roots().await)
        .with_no_client_auth();
    Ok(TlsConnector::from(Arc::new(config)))
}

/// Opens a verified TLS connection to `host:port`.
pub(crate) async fn connect(
    connector: &TlsConnector,
    host: &str,
    port: u16,
) -> Result<Connection, String> {
    let permit = socket_permit().await;
    let name = ServerName::try_from(host.to_owned()).map_err(|error| error.to_string())?;
    let tcp = timeout(CONNECT_WAIT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| format!("{host} did not answer"))?
        .map_err(|error| format!("Could not reach {host}: {error}"))?;
    let stream = timeout(CONNECT_WAIT, connector.connect(name, tcp))
        .await
        .map_err(|_| format!("{host} did not finish the TLS handshake"))?
        .map_err(|error| format!("TLS to {host} failed: {error}"))?;
    Ok(Connection {
        stream,
        _permit: permit,
    })
}

/// An answer to a GET, whatever its status.
pub(crate) struct Response {
    pub status: u16,
    /// How long the server asked to wait before asking again, from `Retry-After`.
    pub retry_after: Option<Duration>,
    pub body: Vec<u8>,
}

/// Fetches `https://host{path}` and returns the status, `Retry-After` and body.
/// HTTP/1.0 is used so the body is never chunked.
pub(crate) async fn get_response(host: &str, path: &str) -> Result<Response, String> {
    let connector = connector().await?;
    let work = async {
        let mut connection = connect(&connector, host, HTTPS_PORT).await?;
        let request = format!(
            "GET {path} HTTP/1.0\r\nHost: {host}\r\nUser-Agent: lanping\r\nAccept: application/json\r\n\r\n"
        );
        connection
            .write_all(request.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        let mut response = Vec::new();
        let read = (&mut *connection)
            .take(MAX_GET_BYTES)
            .read_to_end(&mut response)
            .await;
        // Many servers close without a TLS close notice once the answer is sent. That ends
        // the answer, and what was read before it is kept.
        if let Err(error) = read
            && error.kind() != io::ErrorKind::UnexpectedEof
        {
            return Err(error.to_string());
        }
        parse_response(host, &response)
    };
    timeout(GET_WAIT, work)
        .await
        .map_err(|_| format!("{host} did not answer in time"))?
}

fn parse_response(host: &str, response: &[u8]) -> Result<Response, String> {
    let end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| format!("{host} sent an unreadable answer"))?;
    let head = String::from_utf8_lossy(&response[..end]);
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .ok_or_else(|| format!("{host} sent an unreadable answer"))?;
    let retry_after = lines.find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.trim()
            .eq_ignore_ascii_case("retry-after")
            .then(|| value.trim().parse::<u64>().ok())
            .flatten()
            .map(Duration::from_secs)
    });
    Ok(Response {
        status,
        retry_after,
        body: response[end + 4..].to_vec(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_status_headers_and_body() {
        let response = parse_response("h", b"HTTP/1.0 200 OK\r\nX: y\r\n\r\n{\"a\":1}").unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, b"{\"a\":1}");
        assert_eq!(response.retry_after, None);
    }

    #[test]
    fn reads_how_long_a_busy_server_asks_to_wait() {
        let response = parse_response(
            "h",
            b"HTTP/1.0 429 Too Many Requests\r\nRetry-After: 7\r\n\r\n",
        )
        .unwrap();
        assert_eq!(response.status, 429);
        assert_eq!(response.retry_after, Some(Duration::from_secs(7)));
    }

    #[test]
    fn garbage_is_an_error() {
        assert!(parse_response("h", b"nonsense").is_err());
        assert!(parse_response("h", b"HTTP/1.0 abc\r\n\r\n").is_err());
    }
}
