//! LAN throughput test between two machines that run LanPing.
//!
//! One machine listens ([`serve`]). The other ([`run_client`]) opens several TCP
//! streams to it and moves data for a few seconds in one direction. The protocol is
//! tiny: a header of the magic bytes, a direction byte and a duration byte, then raw data.

use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, mpsc};
use tokio::time::{sleep, timeout};

/// Port the test server listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 5252;
const MAGIC: &[u8; 5] = b"LPNG1";
const DOWNLOAD: u8 = b'D';
const UPLOAD: u8 = b'U';
const HEADER_LENGTH: usize = MAGIC.len() + 2;
/// Streams used in parallel, so one slow stream does not cap a fast link.
const STREAMS: usize = 4;
const CHUNK_BYTES: usize = 128 * 1024;
/// Longest test a server agrees to run.
pub const MAX_SECONDS: u8 = 30;
/// Longest a server waits for a client's header.
const HEADER_WAIT: Duration = Duration::from_secs(5);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
/// Extra time allowed on top of the test length, for the last bytes to drain.
const GRACE: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: usize = 16;
pub(crate) const BITS_PER_MEGABIT: f64 = 1_000_000.0;
const SAMPLE_EVERY: Duration = Duration::from_secs(1);

/// Which way the data goes, as seen from the machine that starts the test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Direction {
    Download,
    Upload,
}

impl Direction {
    fn byte(self) -> u8 {
        match self {
            Self::Download => DOWNLOAD,
            Self::Upload => UPLOAD,
        }
    }
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Download => "Download",
            Self::Upload => "Upload",
        })
    }
}

/// What one test measured.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThroughputResult {
    pub direction: Direction,
    pub bytes: u64,
    pub seconds: f64,
    /// Megabits per second over the whole test.
    pub mbps: f64,
    /// Megabits per second in each second of the test, for a graph.
    pub samples_mbps: Vec<f64>,
}

impl ThroughputResult {
    pub fn new(direction: Direction, bytes: u64, seconds: f64, samples_mbps: Vec<f64>) -> Self {
        let mbps = if seconds > 0.0 {
            bytes as f64 * 8.0 / seconds / BITS_PER_MEGABIT
        } else {
            0.0
        };
        Self {
            direction,
            bytes,
            seconds,
            mbps,
            samples_mbps,
        }
    }
}

/// A test the server took part in. A test that uses several streams gives several of these.
#[derive(Debug, Clone, PartialEq)]
pub struct ServedStream {
    pub peer: SocketAddr,
    pub direction: Direction,
    pub bytes: u64,
}

/// The addresses other machines can use to reach this one, without loopback.
pub fn local_addresses() -> Vec<IpAddr> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|interface| interface.ip())
        .filter(|address| !address.is_loopback() && address.is_ipv4())
        .collect()
}

/// Starts listening for tests on `port`. Port 0 picks a free one.
pub async fn bind(port: u16) -> io::Result<TcpListener> {
    TcpListener::bind(("0.0.0.0", port)).await
}

/// Answers tests until the future is dropped. Each finished stream is reported on `served`.
pub async fn serve(listener: TcpListener, served: mpsc::UnboundedSender<ServedStream>) {
    let slots = Arc::new(Semaphore::new(MAX_CONNECTIONS));
    while let Ok((stream, peer)) = listener.accept().await {
        let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
            continue;
        };
        let served = served.clone();
        tokio::spawn(async move {
            if let Some(stream) = handle(stream, peer).await {
                let _ = served.send(stream);
            }
            drop(slot);
        });
    }
}

/// Reads the header and carries out one stream. `None` when the client did not speak the protocol.
async fn handle(mut stream: TcpStream, peer: SocketAddr) -> Option<ServedStream> {
    let mut header = [0; HEADER_LENGTH];
    timeout(HEADER_WAIT, stream.read_exact(&mut header))
        .await
        .ok()?
        .ok()?;
    if &header[..MAGIC.len()] != MAGIC {
        return None;
    }
    let seconds = header[HEADER_LENGTH - 1].clamp(1, MAX_SECONDS);
    let duration = Duration::from_secs(u64::from(seconds));
    match header[MAGIC.len()] {
        DOWNLOAD => {
            let bytes = send_for(&mut stream, duration).await;
            Some(ServedStream {
                peer,
                direction: Direction::Download,
                bytes,
            })
        }
        UPLOAD => {
            let bytes = receive_until_closed(&mut stream, duration + GRACE).await;
            let _ = stream.write_all(&bytes.to_be_bytes()).await;
            Some(ServedStream {
                peer,
                direction: Direction::Upload,
                bytes,
            })
        }
        _ => None,
    }
}

/// Writes zeros for `duration`, then closes. Returns the bytes written.
async fn send_for(stream: &mut TcpStream, duration: Duration) -> u64 {
    let chunk = vec![0; CHUNK_BYTES];
    let deadline = Instant::now() + duration;
    let mut sent = 0;
    while Instant::now() < deadline {
        if stream.write_all(&chunk).await.is_err() {
            break;
        }
        sent += chunk.len() as u64;
    }
    let _ = stream.shutdown().await;
    sent
}

/// Reads until the other side closes its writing half or `limit` passes.
async fn receive_until_closed(stream: &mut TcpStream, limit: Duration) -> u64 {
    let mut buffer = vec![0; CHUNK_BYTES];
    let deadline = Instant::now() + limit;
    let mut received = 0;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match timeout(left, stream.read(&mut buffer)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
            Ok(Ok(length)) => received += length as u64,
        }
    }
    received
}

/// Runs a test against `host:port` and returns what it measured. `progress` gets the
/// megabits per second of each finished second while the test runs.
pub async fn run_client(
    host: &str,
    port: u16,
    direction: Direction,
    seconds: u8,
    progress: Option<mpsc::UnboundedSender<f64>>,
) -> Result<ThroughputResult, String> {
    let seconds = seconds.clamp(1, MAX_SECONDS);
    let duration = Duration::from_secs(u64::from(seconds));
    let counter = Arc::new(AtomicU64::new(0));
    let started = Instant::now();
    let samples = Arc::new(Mutex::new(Vec::new()));
    let sampler = tokio::spawn(sample(Arc::clone(&counter), Arc::clone(&samples), progress));

    let mut streams = tokio::task::JoinSet::new();
    for _ in 0..STREAMS {
        let (host, counter) = (host.to_owned(), Arc::clone(&counter));
        streams.spawn(async move {
            match direction {
                Direction::Download => download_stream(&host, port, seconds, &counter).await,
                Direction::Upload => upload_stream(&host, port, seconds, duration, &counter).await,
            }
        });
    }
    let mut bytes = 0;
    let mut failure = None;
    while let Some(joined) = streams.join_next().await {
        match joined {
            Ok(Ok(stream_bytes)) => bytes += stream_bytes,
            Ok(Err(error)) => failure = Some(error),
            Err(error) => failure = Some(error.to_string()),
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    sampler.abort();
    if bytes == 0 {
        return Err(failure.unwrap_or_else(|| "No data was transferred".to_owned()));
    }
    let samples = samples
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(ThroughputResult::new(direction, bytes, elapsed, samples))
}

/// Samples the byte counter once a second until it is stopped. Keeps the megabits per
/// second of each second in `samples` and reports it on `progress`.
pub(crate) async fn sample(
    counter: Arc<AtomicU64>,
    samples: Arc<Mutex<Vec<f64>>>,
    progress: Option<mpsc::UnboundedSender<f64>>,
) {
    let mut last = 0;
    loop {
        sleep(SAMPLE_EVERY).await;
        let now = counter.load(Ordering::Relaxed);
        let mbps = (now - last) as f64 * 8.0 / SAMPLE_EVERY.as_secs_f64() / BITS_PER_MEGABIT;
        last = now;
        samples
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(mbps);
        if let Some(progress) = &progress {
            let _ = progress.send(mbps);
        }
    }
}

async fn connect(
    host: &str,
    port: u16,
    direction: Direction,
    seconds: u8,
) -> Result<TcpStream, String> {
    let mut stream = timeout(CONNECT_WAIT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| {
            "The test server did not answer. Is LanPing serving there, and is the port open?"
                .to_owned()
        })?
        .map_err(|error| match error.kind() {
            io::ErrorKind::ConnectionRefused => format!(
                "Nothing is listening on port {port}. Start the test server on that machine."
            ),
            _ => format!("Could not connect: {error}"),
        })?;
    let mut header = [0; HEADER_LENGTH];
    header[..MAGIC.len()].copy_from_slice(MAGIC);
    header[MAGIC.len()] = direction.byte();
    header[HEADER_LENGTH - 1] = seconds;
    stream
        .write_all(&header)
        .await
        .map_err(|error| format!("Could not start the test: {error}"))?;
    Ok(stream)
}

/// Reads what the server sends and counts it. Returns the bytes received.
async fn download_stream(
    host: &str,
    port: u16,
    seconds: u8,
    counter: &AtomicU64,
) -> Result<u64, String> {
    let mut stream = connect(host, port, Direction::Download, seconds).await?;
    let mut buffer = vec![0; CHUNK_BYTES];
    let deadline = Instant::now() + Duration::from_secs(u64::from(seconds)) + GRACE;
    let mut received = 0;
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match timeout(left, stream.read(&mut buffer)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Err(error)) => return Err(format!("The connection broke: {error}")),
            Ok(Ok(length)) => {
                received += length as u64;
                counter.fetch_add(length as u64, Ordering::Relaxed);
            }
        }
    }
    Ok(received)
}

/// Writes for the test length, then asks the server how much arrived. Returns that count.
async fn upload_stream(
    host: &str,
    port: u16,
    seconds: u8,
    duration: Duration,
    counter: &AtomicU64,
) -> Result<u64, String> {
    let mut stream = connect(host, port, Direction::Upload, seconds).await?;
    let chunk = vec![0; CHUNK_BYTES];
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        stream
            .write_all(&chunk)
            .await
            .map_err(|error| format!("The connection broke: {error}"))?;
        counter.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    stream
        .shutdown()
        .await
        .map_err(|error| format!("The connection broke: {error}"))?;
    let mut reply = [0; 8];
    timeout(GRACE, stream.read_exact(&mut reply))
        .await
        .map_err(|_| "The server did not report how much it received".to_owned())?
        .map_err(|error| format!("The connection broke: {error}"))?;
    Ok(u64::from_be_bytes(reply))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn start_server() -> (u16, mpsc::UnboundedReceiver<ServedStream>) {
        let listener = bind(0).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (served, events) = mpsc::unbounded_channel();
        tokio::spawn(serve(listener, served));
        (port, events)
    }

    #[test]
    fn computes_megabits_per_second() {
        let result = ThroughputResult::new(Direction::Download, 125_000_000, 2.0, vec![]);
        assert_eq!(result.mbps, 500.0);
        assert_eq!(
            ThroughputResult::new(Direction::Upload, 5, 0.0, vec![]).mbps,
            0.0
        );
    }

    #[tokio::test]
    async fn measures_a_download_and_the_server_sees_it() {
        let (port, mut events) = start_server().await;
        let result = run_client("127.0.0.1", port, Direction::Download, 1, None)
            .await
            .unwrap();
        assert!(result.bytes > 0 && result.mbps > 0.0);
        assert_eq!(result.direction, Direction::Download);
        let served = events.recv().await.unwrap();
        assert_eq!(served.direction, Direction::Download);
        assert!(served.peer.ip().is_loopback());
    }

    #[tokio::test]
    async fn measures_an_upload_from_what_the_server_received() {
        let (port, mut events) = start_server().await;
        let (progress, mut ticks) = mpsc::unbounded_channel();
        let result = run_client("127.0.0.1", port, Direction::Upload, 2, Some(progress))
            .await
            .unwrap();
        assert!(result.bytes > 0 && result.mbps > 0.0);
        let served = events.recv().await.unwrap();
        assert_eq!(served.direction, Direction::Upload);
        assert!(served.bytes > 0);
        assert!(
            ticks.recv().await.is_some(),
            "progress is reported each second"
        );
        assert!(!result.samples_mbps.is_empty());
    }

    #[tokio::test]
    async fn a_stranger_that_does_not_speak_the_protocol_is_dropped() {
        let (port, mut events) = start_server().await;
        let mut stranger = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        stranger.write_all(b"GET / HTTP/1.0\r\n\r\n").await.unwrap();
        let mut reply = Vec::new();
        let _ = stranger.read_to_end(&mut reply).await;
        assert!(reply.is_empty());
        assert!(events.try_recv().is_err());
        // The server still works afterwards.
        assert!(
            run_client("127.0.0.1", port, Direction::Download, 1, None)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn explains_a_closed_port() {
        let port = {
            let listener = bind(0).await.unwrap();
            listener.local_addr().unwrap().port()
        };
        let error = run_client("127.0.0.1", port, Direction::Download, 1, None)
            .await
            .unwrap_err();
        assert!(error.contains("Nothing is listening"), "{error}");
    }

    #[test]
    fn local_addresses_never_include_loopback() {
        assert!(
            local_addresses()
                .iter()
                .all(|address| !address.is_loopback())
        );
    }
}
