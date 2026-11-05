//! Internet speed test against Cloudflare's public speed endpoint, over HTTPS.
//!
//! Each run sends real traffic to `speed.cloudflare.com`, which can see the caller's
//! address. Nothing else is sent: the upload is zeros.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tokio_rustls::TlsConnector;

use crate::https::{self, Connection};
use crate::throughput::{Direction, ThroughputResult, sample};

/// The service the test talks to, shown to the user before they run it.
pub const SERVICE_HOST: &str = "speed.cloudflare.com";
/// Bytes asked for in one download request. The service refuses 100 MB and more.
const DOWNLOAD_BYTES: u64 = 90_000_000;
/// Bytes promised in one upload request.
const UPLOAD_BYTES: u64 = 25_000_000;
const STREAMS: usize = 4;
const CHUNK_BYTES: usize = 64 * 1024;
const MAX_HEADER_BYTES: usize = 16 * 1024;
const MIN_SECONDS: u8 = 3;
const MAX_SECONDS: u8 = 30;

/// Measures download or upload speed for about `seconds`. `progress` gets the megabits per
/// second of each finished second.
pub async fn run(
    direction: Direction,
    seconds: u8,
    progress: Option<mpsc::UnboundedSender<f64>>,
) -> Result<ThroughputResult, String> {
    let seconds = seconds.clamp(MIN_SECONDS, MAX_SECONDS);
    let connector = https::connector().await?;
    let counter = Arc::new(AtomicU64::new(0));
    let samples = Arc::new(Mutex::new(Vec::new()));
    let started = Instant::now();
    let deadline = started + Duration::from_secs(u64::from(seconds));
    let sampler = tokio::spawn(sample(Arc::clone(&counter), Arc::clone(&samples), progress));

    let mut streams = tokio::task::JoinSet::new();
    for _ in 0..STREAMS {
        let (connector, counter) = (connector.clone(), Arc::clone(&counter));
        streams.spawn(async move {
            match direction {
                Direction::Download => download(&connector, deadline, &counter).await,
                Direction::Upload => upload(&connector, deadline, &counter).await,
            }
        });
    }
    let mut failure = None;
    while let Some(joined) = streams.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(error)) => failure = Some(error),
            Err(error) => failure = Some(error.to_string()),
        }
    }
    sampler.abort();
    let bytes = counter.load(Ordering::Relaxed);
    if bytes == 0 {
        return Err(failure.unwrap_or_else(|| "No data was transferred".to_owned()));
    }
    let samples = samples
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Ok(ThroughputResult::new(
        direction,
        bytes,
        started.elapsed().as_secs_f64(),
        samples,
    ))
}

async fn open(connector: &TlsConnector) -> Result<Connection, String> {
    https::connect(connector, SERVICE_HOST, https::HTTPS_PORT).await
}

/// Requests files one after another and counts their bodies until `deadline`.
async fn download(
    connector: &TlsConnector,
    deadline: Instant,
    counter: &AtomicU64,
) -> Result<(), String> {
    while Instant::now() < deadline {
        let mut stream = open(connector).await?;
        let request = format!(
            "GET /__down?bytes={DOWNLOAD_BYTES} HTTP/1.1\r\nHost: {SERVICE_HOST}\r\nUser-Agent: lanping\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        let mut body = BodyCounter::default();
        let mut buffer = vec![0; CHUNK_BYTES];
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match timeout(left, stream.read(&mut buffer)).await {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Err(error)) => return Err(format!("The connection broke: {error}")),
                Ok(Ok(length)) => {
                    let bytes = body.feed(&buffer[..length])?;
                    counter.fetch_add(bytes as u64, Ordering::Relaxed);
                }
            }
        }
    }
    Ok(())
}

/// Posts zeros one request after another and counts what was written until `deadline`.
async fn upload(
    connector: &TlsConnector,
    deadline: Instant,
    counter: &AtomicU64,
) -> Result<(), String> {
    let chunk = vec![0; CHUNK_BYTES];
    while Instant::now() < deadline {
        let mut stream = open(connector).await?;
        let request = format!(
            "POST /__up HTTP/1.1\r\nHost: {SERVICE_HOST}\r\nUser-Agent: lanping\r\nContent-Type: application/octet-stream\r\nContent-Length: {UPLOAD_BYTES}\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|error| error.to_string())?;
        let mut sent = 0;
        while sent < UPLOAD_BYTES && Instant::now() < deadline {
            let length = chunk.len().min((UPLOAD_BYTES - sent) as usize);
            stream
                .write_all(&chunk[..length])
                .await
                .map_err(|error| format!("The connection broke: {error}"))?;
            sent += length as u64;
            counter.fetch_add(length as u64, Ordering::Relaxed);
        }
    }
    Ok(())
}

/// Skips the HTTP response header and counts only the body bytes after it.
#[derive(Default)]
struct BodyCounter {
    header: Vec<u8>,
    header_done: bool,
}

impl BodyCounter {
    /// Returns how many bytes of `chunk` belong to the body. Fails when the status is not 200.
    fn feed(&mut self, chunk: &[u8]) -> Result<usize, String> {
        if self.header_done {
            return Ok(chunk.len());
        }
        let already = self.header.len();
        self.header.extend_from_slice(chunk);
        let Some(end) = self
            .header
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
        else {
            if self.header.len() > MAX_HEADER_BYTES {
                return Err("The server sent an unreadable answer".to_owned());
            }
            return Ok(0);
        };
        self.header_done = true;
        let status = String::from_utf8_lossy(&self.header[..end])
            .lines()
            .next()
            .unwrap_or_default()
            .to_owned();
        if !status
            .split_whitespace()
            .nth(1)
            .is_some_and(|code| code == "200")
        {
            return Err(format!("{SERVICE_HOST} answered: {status}"));
        }
        let body_start = end + 4;
        Ok((already + chunk.len()).saturating_sub(body_start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_only_the_body_after_the_header() {
        let mut counter = BodyCounter::default();
        assert_eq!(
            counter
                .feed(b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\n\r\n12345")
                .unwrap(),
            5
        );
        assert_eq!(counter.feed(b"6789").unwrap(), 4);
    }

    #[test]
    fn finds_a_header_split_across_chunks() {
        let mut counter = BodyCounter::default();
        assert_eq!(counter.feed(b"HTTP/1.1 200 OK\r\nX: y\r").unwrap(), 0);
        assert_eq!(counter.feed(b"\n\r\nabc").unwrap(), 3);
    }

    #[test]
    fn a_body_that_starts_with_the_header_end_counts_in_full() {
        let mut counter = BodyCounter::default();
        assert_eq!(counter.feed(b"HTTP/1.1 200 OK\r\n\r\n").unwrap(), 0);
        assert_eq!(counter.feed(&[0; 100]).unwrap(), 100);
    }

    #[test]
    fn a_status_other_than_200_is_an_error() {
        let mut counter = BodyCounter::default();
        let error = counter
            .feed(b"HTTP/1.1 429 Too Many Requests\r\n\r\n")
            .unwrap_err();
        assert!(error.contains("429"), "{error}");
    }

    #[test]
    fn an_endless_header_is_refused() {
        let mut counter = BodyCounter::default();
        assert!(counter.feed(&vec![b'a'; MAX_HEADER_BYTES + 1]).is_err());
    }
}
