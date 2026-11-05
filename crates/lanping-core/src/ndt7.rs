//! M-Lab NDT7 speed test: servers all around the world, found through M-Lab's locate
//! service, and an open measurement protocol (WebSocket over TLS).
//!
//! Each test sends real traffic to a server run by M-Lab or a partner, which can see the
//! caller's address. Results are measured at the application level.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::https::{self, HTTPS_PORT};
use crate::throughput::{Direction, ThroughputResult, sample};
use crate::websocket::{self, Message};

/// Where servers are looked up.
pub const LOCATE_HOST: &str = "locate.measurementlab.net";
const LOCATE_PATH: &str = "/v2/nearest/ndt/ndt7";
const SUBPROTOCOL: &str = "net.measurementlab.ndt.v7";
const DOWNLOAD_KEY: &str = "wss:///ndt/v7/download";
const UPLOAD_KEY: &str = "wss:///ndt/v7/upload";
const UPLOAD_MESSAGE_BYTES: usize = 128 * 1024;
const MIN_SECONDS: u8 = 3;
/// NDT7 servers end a test after about ten seconds.
const MAX_SECONDS: u8 = 10;
/// How long after closing an upload the server's last measurement is waited for.
const LAST_MEASUREMENT_WAIT: Duration = Duration::from_secs(2);
const MICROSECONDS_PER_SECOND: f64 = 1_000_000.0;

/// Which servers to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// The ones nearest to this computer.
    Nearest,
    /// The ones in a country, by its two letter code such as `JP`.
    Country(String),
}

impl Location {
    fn query(&self) -> String {
        match self {
            Self::Nearest => String::new(),
            Self::Country(code) => format!("?country={code}"),
        }
    }
}

/// A server that can run a test, with the addresses to use. The addresses carry a token
/// that stops working after a couple of minutes, so ask for servers shortly before a test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub machine: String,
    pub city: String,
    pub country: String,
    download_url: String,
    upload_url: String,
}

impl Server {
    pub fn new(
        machine: &str,
        city: &str,
        country: &str,
        download_url: &str,
        upload_url: &str,
    ) -> Self {
        Self {
            machine: machine.to_owned(),
            city: city.to_owned(),
            country: country.to_owned(),
            download_url: download_url.to_owned(),
            upload_url: upload_url.to_owned(),
        }
    }

    /// The host and port a test connects to, which is also what a ping measures.
    pub fn address(&self) -> Result<(String, u16), String> {
        split_wss_url(&self.download_url).map(|(host, port, _)| (host, port))
    }

    /// `Tokyo, JP`, or just the country when the city is unknown.
    pub fn place(&self) -> String {
        if self.city.is_empty() {
            self.country.clone()
        } else {
            format!("{}, {}", self.city, self.country)
        }
    }
}

impl fmt::Display for Server {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let host = self.machine.split('.').next().unwrap_or(&self.machine);
        write!(f, "{} ({host})", self.place())
    }
}

#[derive(Deserialize)]
struct LocateResponse {
    #[serde(default)]
    results: Vec<LocateResult>,
}

#[derive(Deserialize)]
struct LocateResult {
    machine: String,
    #[serde(default)]
    location: Place,
    #[serde(default)]
    urls: HashMap<String, String>,
}

#[derive(Deserialize, Default)]
struct Place {
    #[serde(default)]
    city: String,
    #[serde(default)]
    country: String,
}

/// Asks M-Lab for test servers in `location`. Servers without both test addresses are left out.
pub async fn locate(location: &Location) -> Result<Vec<Server>, String> {
    let path = format!("{LOCATE_PATH}{}", location.query());
    let body = https::get(LOCATE_HOST, &path).await?;
    parse_servers(&body)
}

fn parse_servers(body: &[u8]) -> Result<Vec<Server>, String> {
    let response: LocateResponse = serde_json::from_slice(body)
        .map_err(|_| "M-Lab sent a list of servers that could not be read".to_owned())?;
    let servers: Vec<Server> = response
        .results
        .into_iter()
        .filter_map(|result| {
            Some(Server {
                machine: result.machine,
                city: result.location.city,
                country: result.location.country,
                download_url: result.urls.get(DOWNLOAD_KEY)?.clone(),
                upload_url: result.urls.get(UPLOAD_KEY)?.clone(),
            })
        })
        .collect();
    if servers.is_empty() {
        return Err("M-Lab has no test server there".to_owned());
    }
    Ok(servers)
}

/// Host, port and path with query of a `wss://` address.
fn split_wss_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url
        .strip_prefix("wss://")
        .ok_or_else(|| "The server gave an address that is not wss://".to_owned())?;
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse()
                .map_err(|_| "The server gave an address with a bad port".to_owned())?,
        ),
        None => (authority, HTTPS_PORT),
    };
    if host.is_empty() {
        return Err("The server gave an address without a host".to_owned());
    }
    Ok((host.to_owned(), port, format!("/{path}")))
}

/// Runs a test against `server` for about `seconds` (3 to 10). `progress` gets the megabits
/// per second of each finished second.
pub async fn run(
    server: &Server,
    direction: Direction,
    seconds: u8,
    progress: Option<mpsc::UnboundedSender<f64>>,
) -> Result<ThroughputResult, String> {
    let url = match direction {
        Direction::Download => &server.download_url,
        Direction::Upload => &server.upload_url,
    };
    let (host, port, path) = split_wss_url(url)?;
    let connector = https::connector().await?;
    let connection = https::connect(&connector, &host, port).await?;
    measure(connection, &host, &path, direction, seconds, progress).await
}

/// The test itself, on any stream that has been connected to the server.
async fn measure<S>(
    stream: S,
    host: &str,
    path: &str,
    direction: Direction,
    seconds: u8,
    progress: Option<mpsc::UnboundedSender<f64>>,
) -> Result<ThroughputResult, String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let seconds = seconds.clamp(MIN_SECONDS, MAX_SECONDS);
    let (reader, writer) = websocket::connect(stream, host, path, SUBPROTOCOL).await?;
    let counter = Arc::new(AtomicU64::new(0));
    let samples = Arc::new(Mutex::new(Vec::new()));
    let sampler = tokio::spawn(sample(Arc::clone(&counter), Arc::clone(&samples), progress));
    let started = Instant::now();
    let deadline = started + Duration::from_secs(u64::from(seconds));
    let outcome = match direction {
        Direction::Download => download(reader, writer, deadline, &counter).await,
        Direction::Upload => upload(reader, writer, deadline, &counter).await,
    };
    sampler.abort();
    let (bytes, elapsed) = outcome?;
    let elapsed = elapsed.unwrap_or_else(|| started.elapsed().as_secs_f64());
    let samples = samples
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(ThroughputResult::new(direction, bytes, elapsed, samples))
}

type Reader<S> = websocket::Reader<tokio::io::ReadHalf<S>>;
type Writer<S> = websocket::Writer<tokio::io::WriteHalf<S>>;

/// Reads what the server sends until `deadline`, a close, or the connection ends.
async fn download<S>(
    mut reader: Reader<S>,
    mut writer: Writer<S>,
    deadline: Instant,
    counter: &AtomicU64,
) -> Result<(u64, Option<f64>), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut failure = None;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        let next = timeout(
            left,
            reader.next_message(|bytes| {
                counter.fetch_add(bytes as u64, Ordering::Relaxed);
            }),
        )
        .await;
        match next {
            Err(_) | Ok(Ok(Message::Close)) => break,
            Ok(Ok(Message::Ping(payload))) => {
                let _ = writer.send_pong(&payload).await;
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                failure = Some(format!("The connection broke: {error}"));
                break;
            }
        }
    }
    let _ = writer.send_close().await;
    let bytes = counter.load(Ordering::Relaxed);
    match failure {
        Some(error) if bytes == 0 => Err(error),
        _ => Ok((bytes, None)),
    }
}

/// What the server has received so far, as it last said: bytes and microseconds.
type ServerView = Arc<Mutex<Option<(u64, f64)>>>;

/// Sends zeros until `deadline`. The bytes and time come from the server's own last
/// measurement when there is one, since it counts what really arrived.
async fn upload<S>(
    mut reader: Reader<S>,
    mut writer: Writer<S>,
    deadline: Instant,
    counter: &AtomicU64,
) -> Result<(u64, Option<f64>), String>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let view: ServerView = Arc::new(Mutex::new(None));
    let mut listener = tokio::spawn({
        let view = Arc::clone(&view);
        async move {
            while let Ok(message) = reader.next_message(|_| {}).await {
                match message {
                    Message::Text(json) => {
                        if let Some(seen) = server_measurement(&json) {
                            *view.lock().unwrap_or_else(PoisonError::into_inner) = Some(seen);
                        }
                    }
                    Message::Close => break,
                    _ => {}
                }
            }
        }
    });
    writer.prepare_zeros(UPLOAD_MESSAGE_BYTES);
    let mut failure = None;
    while Instant::now() < deadline {
        match writer.send_prepared().await {
            Ok(sent) => {
                counter.fetch_add(sent as u64, Ordering::Relaxed);
            }
            Err(error) => {
                failure = Some(format!("The connection broke: {error}"));
                break;
            }
        }
    }
    let _ = writer.send_close().await;
    let _ = timeout(LAST_MEASUREMENT_WAIT, &mut listener).await;
    listener.abort();
    let sent = counter.load(Ordering::Relaxed);
    let seen = *view.lock().unwrap_or_else(PoisonError::into_inner);
    match (seen, failure) {
        (Some((bytes, seconds)), _) => Ok((bytes, Some(seconds))),
        (None, Some(error)) if sent == 0 => Err(error),
        (None, _) => Ok((sent, None)),
    }
}

/// Bytes and seconds from a server measurement message, when it carries them.
fn server_measurement(json: &str) -> Option<(u64, f64)> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    let app = value.get("AppInfo")?;
    let bytes = app.get("NumBytes")?.as_u64()?;
    let microseconds = app.get("ElapsedTime")?.as_f64()?;
    (microseconds > 0.0).then(|| (bytes, microseconds / MICROSECONDS_PER_SECOND))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, duplex};

    const LOCATE_JSON: &str = r#"{"results":[
        {"machine":"ndt-hel1.example.org","location":{"city":"Helsinki","country":"FI"},
         "urls":{"wss:///ndt/v7/download":"wss://ndt-hel1.example.org/ndt/v7/download?access_token=a",
                 "wss:///ndt/v7/upload":"wss://ndt-hel1.example.org/ndt/v7/upload?access_token=b"}},
        {"machine":"half.example.org","location":{"city":"X","country":"XX"},
         "urls":{"wss:///ndt/v7/download":"wss://half.example.org/d"}},
        {"machine":"ndt-tyo.example.org","location":{"country":"JP"},
         "urls":{"wss:///ndt/v7/download":"wss://ndt-tyo.example.org/d","wss:///ndt/v7/upload":"wss://ndt-tyo.example.org/u"}}
    ]}"#;

    #[test]
    fn reads_servers_and_leaves_out_ones_without_both_addresses() {
        let servers = parse_servers(LOCATE_JSON.as_bytes()).unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0].place(), "Helsinki, FI");
        assert_eq!(servers[0].to_string(), "Helsinki, FI (ndt-hel1)");
        assert_eq!(servers[1].place(), "JP");
        assert!(servers[0].download_url.contains("access_token=a"));
    }

    #[test]
    fn an_empty_or_broken_list_is_an_error() {
        assert!(parse_servers(br#"{"results":[]}"#).is_err());
        assert!(parse_servers(br#"{"error":{"title":"x"}}"#).is_err());
        assert!(parse_servers(b"nonsense").is_err());
    }

    #[test]
    fn builds_the_locate_query_for_a_country() {
        assert_eq!(Location::Nearest.query(), "");
        assert_eq!(Location::Country("JP".to_owned()).query(), "?country=JP");
    }

    #[test]
    fn splits_wss_addresses() {
        assert_eq!(
            split_wss_url("wss://ndt.example.org/ndt/v7/download?access_token=x").unwrap(),
            (
                "ndt.example.org".to_owned(),
                443,
                "/ndt/v7/download?access_token=x".to_owned()
            )
        );
        assert_eq!(
            split_wss_url("wss://h:8443/p").unwrap(),
            ("h".to_owned(), 8443, "/p".to_owned())
        );
        assert_eq!(split_wss_url("wss://h").unwrap().2, "/");
        for bad in ["ws://h/p", "wss:///p", "wss://h:x/p", "https://h/p"] {
            assert!(split_wss_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reads_the_servers_own_byte_count() {
        let json = r#"{"AppInfo":{"ElapsedTime":2000000,"NumBytes":25000000},"Origin":"server"}"#;
        assert_eq!(server_measurement(json), Some((25_000_000, 2.0)));
        assert_eq!(
            server_measurement(r#"{"AppInfo":{"ElapsedTime":0,"NumBytes":1}}"#),
            None
        );
        assert_eq!(server_measurement(r#"{"TCPInfo":{}}"#), None);
        assert_eq!(server_measurement("not json"), None);
    }

    /// Plays the server side of the handshake and returns the stream for the test to use.
    async fn accept_handshake(server: &mut tokio::io::DuplexStream) {
        let mut request = Vec::new();
        let mut buffer = [0; 512];
        while !request.windows(4).any(|window| window == b"\r\n\r\n") {
            let length = server.read(&mut buffer).await.unwrap();
            request.extend_from_slice(&buffer[..length]);
        }
        server
            .write_all(b"HTTP/1.1 101 Switching Protocols\r\n\r\n")
            .await
            .unwrap();
    }

    fn binary_frame(size: usize) -> Vec<u8> {
        let mut frame = vec![0x82, 127];
        frame.extend((size as u64).to_be_bytes());
        frame.extend(vec![0; size]);
        frame
    }

    #[tokio::test]
    async fn measures_a_download_from_binary_messages_until_the_server_closes() {
        let (client, mut server) = duplex(1 << 20);
        tokio::spawn(async move {
            accept_handshake(&mut server).await;
            for _ in 0..5 {
                server.write_all(&binary_frame(100_000)).await.unwrap();
            }
            server.write_all(&[0x88, 2, 3, 232]).await.unwrap();
            let mut rest = Vec::new();
            let _ = server.read_to_end(&mut rest).await;
        });
        let result = measure(client, "h", "/d", Direction::Download, 3, None)
            .await
            .unwrap();
        assert_eq!(result.bytes, 500_000);
        assert_eq!(result.direction, Direction::Download);
        assert!(result.mbps > 0.0);
    }

    #[tokio::test]
    async fn measures_an_upload_from_the_servers_own_count() {
        let (client, mut server) = duplex(1 << 22);
        tokio::spawn(async move {
            accept_handshake(&mut server).await;
            let measurement =
                br#"{"AppInfo":{"ElapsedTime":2500000,"NumBytes":31250000},"Origin":"server"}"#;
            let mut frame = vec![0x81, measurement.len() as u8];
            frame.extend(measurement);
            server.write_all(&frame).await.unwrap();
            let mut sink = vec![0; 1 << 16];
            while server.read(&mut sink).await.is_ok_and(|length| length > 0) {}
        });
        let result = measure(client, "h", "/u", Direction::Upload, 3, None)
            .await
            .unwrap();
        assert_eq!(result.bytes, 31_250_000);
        assert_eq!(result.seconds, 2.5);
        assert_eq!(result.mbps, 100.0);
    }

    #[tokio::test]
    async fn a_download_that_breaks_at_once_is_an_error() {
        let (client, mut server) = duplex(1 << 16);
        tokio::spawn(async move {
            accept_handshake(&mut server).await;
            drop(server);
        });
        let error = measure(client, "h", "/d", Direction::Download, 3, None)
            .await
            .unwrap_err();
        assert!(error.contains("broke"), "{error}");
    }
}
