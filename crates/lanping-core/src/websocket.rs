//! A minimal WebSocket client (RFC 6455), enough for the M-Lab NDT7 speed test: one
//! handshake with a subprotocol, binary and text messages, no extensions.

use std::io;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
    ReadHalf, WriteHalf,
};

const OPCODE_CONTINUATION: u8 = 0;
const OPCODE_TEXT: u8 = 1;
const OPCODE_BINARY: u8 = 2;
const OPCODE_CLOSE: u8 = 8;
const OPCODE_PING: u8 = 9;
const OPCODE_PONG: u8 = 10;
const FLAG_FIN: u8 = 0x80;
const FLAG_MASK: u8 = 0x80;
const CLOSE_NORMAL: [u8; 2] = 1000_u16.to_be_bytes();
const LEN_16_BITS: u8 = 126;
const LEN_64_BITS: u8 = 127;
const MAX_HEADER_LINES: usize = 64;
/// Most bytes the handshake may take, headers included.
const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;
/// Largest text or control message read into memory.
const MAX_SMALL_MESSAGE: u64 = 1024 * 1024;
const DISCARD_CHUNK: usize = 64 * 1024;
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// What arrived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Message {
    Text(String),
    /// A binary message of this many bytes. Its content is not kept.
    Binary(u64),
    Ping(Vec<u8>),
    /// The answer to a ping of ours. Nothing to do with it.
    Pong,
    Close,
}

pub(crate) struct Reader<R> {
    inner: BufReader<R>,
    /// Where binary messages are read to and dropped; made once, on first use.
    scratch: Vec<u8>,
}

pub(crate) struct Writer<W> {
    inner: W,
    mask: [u8; 4],
    /// A ready frame of zeros, set by [`Writer::prepare_zeros`].
    prepared: Vec<u8>,
}

/// Reads one line of the handshake, spending from `budget`. A server that never ends a line,
/// or never ends its headers, runs the budget out instead of the client's memory. The stream
/// ending before the headers do is an error too.
async fn read_capped_line<R: AsyncBufRead + Unpin>(
    reader: &mut R,
    budget: &mut usize,
) -> Result<String, String> {
    let mut line = String::new();
    let mut limited = reader.take(*budget as u64);
    let length = limited
        .read_line(&mut line)
        .await
        .map_err(|error| format!("The WebSocket handshake broke: {error}"))?;
    *budget = budget.saturating_sub(length);
    if length == 0 || !line.ends_with('\n') {
        return Err("The WebSocket handshake ended too soon or ran too long".to_owned());
    }
    Ok(line)
}

/// Does the opening handshake on `stream` for `wss://host{path}` asking for `protocol`, and
/// splits the stream into a reader and a writer.
pub(crate) async fn connect<S>(
    stream: S,
    host: &str,
    path: &str,
    protocol: &str,
) -> Result<(Reader<ReadHalf<S>>, Writer<WriteHalf<S>>), String>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    let key = base64(&random_bytes::<16>());
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: {protocol}\r\nUser-Agent: lanping\r\n\r\n"
    );
    write
        .write_all(request.as_bytes())
        .await
        .map_err(|error| format!("Could not start the WebSocket: {error}"))?;
    let mut budget = MAX_HANDSHAKE_BYTES;
    let status = read_capped_line(&mut reader, &mut budget).await?;
    if status.split_whitespace().nth(1) != Some("101") {
        return Err(format!("{host} refused the WebSocket: {}", status.trim()));
    }
    for _ in 0..MAX_HEADER_LINES {
        let line = read_capped_line(&mut reader, &mut budget).await?;
        if line.trim().is_empty() {
            return Ok((
                Reader {
                    inner: reader,
                    scratch: Vec::new(),
                },
                Writer {
                    inner: write,
                    mask: random_bytes::<4>(),
                    prepared: Vec::new(),
                },
            ));
        }
    }
    Err("The WebSocket handshake never ended".to_owned())
}

impl<R: AsyncRead + Unpin> Reader<R> {
    /// Reads the next message. The bytes of a binary message are counted through `on_bytes`
    /// as they arrive and then dropped. Text and control messages are returned whole.
    pub(crate) async fn next_message(
        &mut self,
        mut on_bytes: impl FnMut(usize),
    ) -> io::Result<Message> {
        let mut head = [0; 2];
        self.inner.read_exact(&mut head).await?;
        let opcode = head[0] & 0x0F;
        let masked = head[1] & FLAG_MASK != 0;
        let length = match head[1] & !FLAG_MASK {
            LEN_16_BITS => {
                let mut bytes = [0; 2];
                self.inner.read_exact(&mut bytes).await?;
                u64::from(u16::from_be_bytes(bytes))
            }
            LEN_64_BITS => {
                let mut bytes = [0; 8];
                self.inner.read_exact(&mut bytes).await?;
                u64::from_be_bytes(bytes)
            }
            short => u64::from(short),
        };
        let mut mask = [0; 4];
        if masked {
            self.inner.read_exact(&mut mask).await?;
        }
        if matches!(opcode, OPCODE_BINARY | OPCODE_CONTINUATION) {
            self.discard(length, &mut on_bytes).await?;
            return Ok(Message::Binary(length));
        }
        if length > MAX_SMALL_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a control or text message was too large",
            ));
        }
        let mut payload = vec![0; length as usize];
        self.inner.read_exact(&mut payload).await?;
        if masked {
            for (index, byte) in payload.iter_mut().enumerate() {
                *byte ^= mask[index % 4];
            }
        }
        Ok(match opcode {
            OPCODE_TEXT => Message::Text(String::from_utf8_lossy(&payload).into_owned()),
            OPCODE_PING => Message::Ping(payload),
            OPCODE_PONG => Message::Pong,
            OPCODE_CLOSE => Message::Close,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "an unknown WebSocket message type",
                ));
            }
        })
    }

    async fn discard(
        &mut self,
        mut length: u64,
        on_bytes: &mut impl FnMut(usize),
    ) -> io::Result<()> {
        if self.scratch.is_empty() {
            self.scratch = vec![0; DISCARD_CHUNK];
        }
        while length > 0 {
            let want = self.scratch.len().min(length as usize);
            let got = self.inner.read(&mut self.scratch[..want]).await?;
            if got == 0 {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            on_bytes(got);
            length -= got as u64;
        }
        Ok(())
    }
}

impl<W: AsyncWrite + Unpin> Writer<W> {
    /// Builds one binary frame of `size` zero bytes, so that sending it again costs nothing.
    pub(crate) fn prepare_zeros(&mut self, size: usize) {
        self.prepared = frame_header(OPCODE_BINARY, size as u64, self.mask);
        // Masking zeros gives the mask itself, repeated.
        self.prepared
            .extend((0..size).map(|index| self.mask[index % 4]));
    }

    /// Sends the frame built by [`Writer::prepare_zeros`]. Returns the payload size.
    pub(crate) async fn send_prepared(&mut self) -> io::Result<usize> {
        self.inner.write_all(&self.prepared).await?;
        Ok(self.prepared.len().saturating_sub(self.header_length()))
    }

    fn header_length(&self) -> usize {
        match self.prepared.get(1).map(|byte| byte & !FLAG_MASK) {
            Some(LEN_16_BITS) => 8,
            Some(LEN_64_BITS) => 14,
            _ => 6,
        }
    }

    pub(crate) async fn send_pong(&mut self, payload: &[u8]) -> io::Result<()> {
        self.send_control(OPCODE_PONG, payload).await
    }

    pub(crate) async fn send_close(&mut self) -> io::Result<()> {
        self.send_control(OPCODE_CLOSE, &CLOSE_NORMAL).await
    }

    async fn send_control(&mut self, opcode: u8, payload: &[u8]) -> io::Result<()> {
        let mut frame = frame_header(opcode, payload.len() as u64, self.mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ self.mask[index % 4]),
        );
        self.inner.write_all(&frame).await?;
        self.inner.flush().await
    }
}

/// The header of a final, masked frame: flags and opcode, length, then the mask.
fn frame_header(opcode: u8, length: u64, mask: [u8; 4]) -> Vec<u8> {
    let mut header = vec![FLAG_FIN | opcode];
    match length {
        0..=125 => header.push(FLAG_MASK | length as u8),
        126..=65_535 => {
            header.push(FLAG_MASK | LEN_16_BITS);
            header.extend((length as u16).to_be_bytes());
        }
        _ => {
            header.push(FLAG_MASK | LEN_64_BITS);
            header.extend(length.to_be_bytes());
        }
    }
    header.extend(mask);
    header
}

fn base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let value = group
            .iter()
            .enumerate()
            .fold(0_u32, |value, (index, byte)| {
                value | u32::from(*byte) << (16 - 8 * index)
            });
        for position in 0..4 {
            if position <= group.len() {
                text.push(char::from(
                    BASE64[(value >> (18 - 6 * position) & 0x3F) as usize],
                ));
            } else {
                text.push('=');
            }
        }
    }
    text
}

/// Bytes that differ from one call to the next. They are not secret: the handshake key and
/// the frame mask only have to be unpredictable enough for proxies not to mistake traffic.
fn random_bytes<const N: usize>() -> [u8; N] {
    static COUNTER: AtomicU32 = AtomicU32::new(0x9E37_79B9);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    let mut state = nanos ^ COUNTER.fetch_add(0x9E37_79B9, Ordering::Relaxed);
    let mut bytes = [0; N];
    for byte in &mut bytes {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        *byte = state as u8;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    /// A server frame: unmasked, final, with the given opcode and payload.
    fn server_frame(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut frame = vec![FLAG_FIN | opcode];
        match payload.len() {
            0..=125 => frame.push(payload.len() as u8),
            126..=65_535 => {
                frame.push(LEN_16_BITS);
                frame.extend((payload.len() as u16).to_be_bytes());
            }
            length => {
                frame.push(LEN_64_BITS);
                frame.extend((length as u64).to_be_bytes());
            }
        }
        frame.extend(payload);
        frame
    }

    async fn reader_over(bytes: Vec<u8>) -> Reader<io::Cursor<Vec<u8>>> {
        Reader {
            inner: BufReader::new(io::Cursor::new(bytes)),
            scratch: Vec::new(),
        }
    }

    #[test]
    fn encodes_base64_with_padding() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"hello world"), "aGVsbG8gd29ybGQ=");
        assert_eq!(base64(b"ab"), "YWI=");
        assert_eq!(base64(&[0; 16]).len(), 24);
    }

    #[test]
    fn random_bytes_change_between_calls() {
        assert_ne!(random_bytes::<16>(), random_bytes::<16>());
    }

    #[tokio::test]
    async fn reads_text_ping_and_close_messages() {
        let mut bytes = server_frame(OPCODE_TEXT, b"{\"a\":1}");
        bytes.extend(server_frame(OPCODE_PING, b"hi"));
        bytes.extend(server_frame(OPCODE_CLOSE, &CLOSE_NORMAL));
        let mut reader = reader_over(bytes).await;
        assert_eq!(
            reader.next_message(|_| {}).await.unwrap(),
            Message::Text("{\"a\":1}".to_owned())
        );
        assert_eq!(
            reader.next_message(|_| {}).await.unwrap(),
            Message::Ping(b"hi".to_vec())
        );
        assert_eq!(reader.next_message(|_| {}).await.unwrap(), Message::Close);
        assert!(reader.next_message(|_| {}).await.is_err());
    }

    #[tokio::test]
    async fn counts_long_binary_messages_as_they_arrive() {
        for size in [100, 70_000, 200_000] {
            let mut reader = reader_over(server_frame(OPCODE_BINARY, &vec![7; size])).await;
            let mut counted = 0;
            let message = reader.next_message(|bytes| counted += bytes).await.unwrap();
            assert_eq!(message, Message::Binary(size as u64));
            assert_eq!(counted, size);
        }
    }

    #[tokio::test]
    async fn unmasks_a_masked_text_message() {
        let mask = [1, 2, 3, 4];
        let masked: Vec<u8> = b"hello"
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4])
            .collect();
        let mut frame = vec![FLAG_FIN | OPCODE_TEXT, FLAG_MASK | 5];
        frame.extend(mask);
        frame.extend(masked);
        let mut reader = reader_over(frame).await;
        assert_eq!(
            reader.next_message(|_| {}).await.unwrap(),
            Message::Text("hello".to_owned())
        );
    }

    #[tokio::test]
    async fn refuses_a_huge_text_message_and_unknown_types() {
        let mut huge = vec![FLAG_FIN | OPCODE_TEXT, LEN_64_BITS];
        huge.extend((MAX_SMALL_MESSAGE + 1).to_be_bytes());
        assert!(reader_over(huge).await.next_message(|_| {}).await.is_err());
        let unknown = server_frame(0x3, b"x");
        assert!(
            reader_over(unknown)
                .await
                .next_message(|_| {})
                .await
                .is_err()
        );
    }

    #[test]
    fn builds_masked_frame_headers_for_each_length_form() {
        let mask = [9, 8, 7, 6];
        assert_eq!(
            frame_header(OPCODE_BINARY, 5, mask),
            [0x82, 0x85, 9, 8, 7, 6]
        );
        assert_eq!(
            frame_header(OPCODE_BINARY, 300, mask),
            [0x82, 0xFE, 1, 44, 9, 8, 7, 6]
        );
        let long = frame_header(OPCODE_BINARY, 70_000, mask);
        assert_eq!(long.len(), 14);
        assert_eq!(long[1], 0xFF);
    }

    #[tokio::test]
    async fn a_prepared_frame_masks_zeros_and_reports_its_payload_size() {
        let (client, mut server) = duplex(1 << 20);
        let mut writer = Writer {
            inner: client,
            mask: [1, 2, 3, 4],
            prepared: Vec::new(),
        };
        for size in [10_usize, 1000, 70_000] {
            writer.prepare_zeros(size);
            assert_eq!(writer.send_prepared().await.unwrap(), size);
        }
        drop(writer);
        let mut sent = Vec::new();
        server.read_to_end(&mut sent).await.unwrap();
        // The first frame: 6 header bytes and ten masked zeros, which equal the mask repeated.
        assert_eq!(&sent[..2], &[0x82, 0x8A]);
        assert_eq!(&sent[6..16], &[1, 2, 3, 4, 1, 2, 3, 4, 1, 2]);
    }

    #[tokio::test]
    async fn the_handshake_asks_for_the_subprotocol_and_then_messages_flow() {
        let (client, mut server) = duplex(64 * 1024);
        let fake_server = tokio::spawn(async move {
            let mut request = Vec::new();
            let mut buffer = [0; 512];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let length = server.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..length]);
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /ndt/v7/download?x=1 HTTP/1.1"));
            assert!(request.contains("Sec-WebSocket-Protocol: net.measurementlab.ndt.v7"));
            assert!(request.contains("Upgrade: websocket"));
            server
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n")
                .await
                .unwrap();
            server
                .write_all(&server_frame(OPCODE_TEXT, b"first"))
                .await
                .unwrap();
        });
        let (mut reader, _writer) = connect(
            client,
            "example.test",
            "/ndt/v7/download?x=1",
            "net.measurementlab.ndt.v7",
        )
        .await
        .unwrap();
        assert_eq!(
            reader.next_message(|_| {}).await.unwrap(),
            Message::Text("first".to_owned())
        );
        fake_server.await.unwrap();
    }

    #[tokio::test]
    async fn a_server_that_never_ends_a_line_is_cut_off() {
        let (client, mut server) = duplex(64 * 1024);
        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            let _ = server.read(&mut buffer).await;
            let _ = server
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\n")
                .await;
            let endless = vec![b'a'; 2 * MAX_HANDSHAKE_BYTES];
            let _ = server.write_all(&endless).await;
        });
        let error = connect(client, "example.test", "/x", "p")
            .await
            .err()
            .unwrap();
        assert!(error.contains("too soon or ran too long"), "{error}");
    }

    #[tokio::test]
    async fn a_server_that_hangs_up_in_the_headers_is_an_error() {
        let (client, mut server) = duplex(64 * 1024);
        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            let _ = server.read(&mut buffer).await;
            let _ = server
                .write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n")
                .await;
        });
        assert!(connect(client, "example.test", "/x", "p").await.is_err());
    }

    #[tokio::test]
    async fn a_pong_is_a_pong_and_not_a_ping() {
        let mut reader = reader_over(server_frame(OPCODE_PONG, b"")).await;
        assert_eq!(reader.next_message(|_| {}).await.unwrap(), Message::Pong);
    }

    #[tokio::test]
    async fn a_refused_handshake_is_an_error_naming_the_status() {
        let (client, mut server) = duplex(64 * 1024);
        tokio::spawn(async move {
            let mut buffer = [0; 1024];
            let _ = server.read(&mut buffer).await;
            let _ = server.write_all(b"HTTP/1.1 403 Forbidden\r\n\r\n").await;
        });
        let error = connect(client, "example.test", "/x", "p")
            .await
            .err()
            .unwrap();
        assert!(error.contains("403"), "{error}");
    }
}
