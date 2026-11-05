//! How a website answers: the redirect chain, where the time goes, the TLS version and
//! which security headers it sends.
//!
//! One ordinary GET per hop of the chain, read as far as the end of the headers. Nothing is
//! posted and no body is downloaded.

use std::fmt;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, lookup_host};
use tokio::time::timeout;

use crate::https::{HTTPS_PORT, connector};
use crate::socket_budget::socket_permit;
use crate::trace::validate_target;
use crate::verdict::Verdict;

const HTTP_PORT: u16 = 80;
const MAX_HOPS: usize = 8;
const STEP_WAIT: Duration = Duration::from_secs(8);
const MAX_HEAD_BYTES: usize = 32 * 1024;
const MAX_HEADERS: usize = 100;
const MAX_VALUE_CHARS: usize = 200;
/// A year-long `max-age` is common; half a year is the least that is worth the header.
const HSTS_MIN_SECONDS: u64 = 15_552_000;
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];

/// Where a request goes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    https: bool,
    host: String,
    port: u16,
    path: String,
}

impl Target {
    fn url(&self) -> String {
        let default = if self.https { HTTPS_PORT } else { HTTP_PORT };
        let port = if self.port == default {
            String::new()
        } else {
            format!(":{}", self.port)
        };
        let scheme = if self.https { "https" } else { "http" };
        format!("{scheme}://{}{port}{}", self.host, self.path)
    }
}

/// `example.com`, `http://example.com:8080/a?b=1` and so on. Without a scheme it is `https`.
fn parse_target(text: &str) -> Result<Target, WebsiteError> {
    let text = text.trim();
    let (https, rest) = match text.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => (true, rest),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => (false, rest),
        Some(_) => return Err(WebsiteError::InvalidUrl),
        None => (true, text),
    };
    let (authority, path) = match rest.find(['/', '?', '#']) {
        Some(at) => rest.split_at(at),
        None => (rest, ""),
    };
    let path = path.split('#').next().unwrap_or_default();
    let path = if path.is_empty() || path.starts_with('?') {
        format!("/{path}")
    } else {
        path.to_owned()
    };
    if path.contains(|c: char| c.is_whitespace() || c.is_control()) {
        return Err(WebsiteError::InvalidUrl);
    }
    let default = if https { HTTPS_PORT } else { HTTP_PORT };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or(WebsiteError::InvalidUrl)?,
        ),
        None => (authority, default),
    };
    let host = validate_target(host).map_err(|_| WebsiteError::InvalidUrl)?;
    Ok(Target {
        https,
        host: host.to_owned(),
        port,
        path,
    })
}

/// Where a `Location` header sends a request that was made to `from`.
fn follow(from: &Target, location: &str) -> Result<Target, WebsiteError> {
    let location = location.trim();
    if location.contains("://") {
        return parse_target(location);
    }
    if let Some(rest) = location.strip_prefix("//") {
        let scheme = if from.https { "https" } else { "http" };
        return parse_target(&format!("{scheme}://{rest}"));
    }
    // `mailto:x` or `javascript:x` start with a scheme, which is not a path to follow.
    if location
        .split(['/', '?'])
        .next()
        .is_some_and(|first| first.contains(':'))
    {
        return Err(WebsiteError::InvalidUrl);
    }
    let path = if location.starts_with('/') {
        location.to_owned()
    } else {
        let directory = from
            .path
            .rsplit_once('/')
            .map_or("", |(directory, _)| directory);
        format!("{directory}/{location}")
    };
    if path.contains(|c: char| c.is_whitespace() || c.is_control()) {
        return Err(WebsiteError::InvalidUrl);
    }
    Ok(Target {
        path,
        ..from.clone()
    })
}

/// One request of the chain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub url: String,
    pub status: u16,
    pub location: Option<String>,
    pub total: Duration,
}

/// Where the time of the last request went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timings {
    pub dns: Duration,
    pub connect: Duration,
    /// Not there for a plain `http://` request.
    pub tls: Option<Duration>,
    /// From sending the request to the first byte back.
    pub first_byte: Duration,
    pub total: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderCheck {
    pub name: &'static str,
    pub value: Option<String>,
    pub verdict: Verdict,
    /// What the header does, or what is wrong with it, in plain words.
    pub advice: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebsiteReport {
    pub hops: Vec<Hop>,
    pub timings: Timings,
    /// Such as `TLS 1.3`. Not there for a plain `http://` request.
    pub tls_version: Option<String>,
    pub headers: Vec<HeaderCheck>,
    /// Why the chain stopped before an answer, when it did.
    pub stopped: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebsiteError {
    InvalidUrl,
    Dns(String),
    Connect(String),
    Tls(String),
    NoAnswer,
    BadReply,
}

impl fmt::Display for WebsiteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl => {
                f.write_str("Enter a web address, such as example.com or https://example.com/page")
            }
            Self::Dns(error) => write!(f, "Could not find the site: {error}"),
            Self::Connect(error) => write!(f, "Could not connect: {error}"),
            Self::Tls(error) => write!(f, "The secure connection failed: {error}"),
            Self::NoAnswer => f.write_str("The site did not answer in time"),
            Self::BadReply => f.write_str("The site sent something that is not a web answer"),
        }
    }
}

/// What one request got.
struct Fetched {
    status: u16,
    headers: Vec<(String, String)>,
    timings: Timings,
    tls_version: Option<String>,
}

/// Visits `address` and follows its redirects. A site that answers with an error status is
/// still a result; a site that cannot be reached at all is an error.
pub async fn check(address: &str) -> Result<WebsiteReport, WebsiteError> {
    let mut target = parse_target(address)?;
    let mut hops = Vec::new();
    let mut stopped = None;
    let mut last = fetch(&target).await?;
    loop {
        let location = header(&last.headers, "location").map(str::to_owned);
        hops.push(Hop {
            url: target.url(),
            status: last.status,
            location: location.clone(),
            total: last.timings.total,
        });
        let Some(location) = location.filter(|_| REDIRECT_STATUSES.contains(&last.status)) else {
            break;
        };
        if hops.len() >= MAX_HOPS {
            stopped = Some(format!("Stopped after {MAX_HOPS} redirects"));
            break;
        }
        let next = match follow(&target, &location) {
            Ok(next) => next,
            Err(_) => {
                stopped = Some(format!(
                    "Redirected to an address that cannot be followed: {location}"
                ));
                break;
            }
        };
        match fetch(&next).await {
            Ok(fetched) => {
                target = next;
                last = fetched;
            }
            Err(error) => {
                stopped = Some(format!(
                    "Could not follow the redirect to {}: {error}",
                    next.url()
                ));
                break;
            }
        }
    }
    Ok(WebsiteReport {
        hops,
        timings: last.timings,
        tls_version: last.tls_version,
        headers: check_headers(&last.headers, target.https),
        stopped,
    })
}

async fn fetch(target: &Target) -> Result<Fetched, WebsiteError> {
    let started = Instant::now();
    let _permit = socket_permit().await;
    let address = resolve(target).await?;
    let dns = started.elapsed();

    let connect_started = Instant::now();
    let tcp = timeout(STEP_WAIT, TcpStream::connect(address))
        .await
        .map_err(|_| WebsiteError::NoAnswer)?
        .map_err(|error| WebsiteError::Connect(error.to_string()))?;
    let connect = connect_started.elapsed();

    let request = format!(
        "GET {} HTTP/1.0\r\nHost: {}\r\nUser-Agent: lanping\r\nAccept: */*\r\nConnection: close\r\n\r\n",
        target.path,
        host_header(target),
    );
    if !target.https {
        let (head, first_byte) = exchange(tcp, &request).await?;
        return finish(&head, dns, connect, None, first_byte, started, None);
    }
    let tls_started = Instant::now();
    let name = ServerName::try_from(target.host.clone())
        .map_err(|error| WebsiteError::Tls(error.to_string()))?;
    let connector = connector().await.map_err(WebsiteError::Tls)?;
    let stream = timeout(STEP_WAIT, connector.connect(name, tcp))
        .await
        .map_err(|_| WebsiteError::NoAnswer)?
        .map_err(|error| WebsiteError::Tls(error.to_string()))?;
    let tls = tls_started.elapsed();
    let version = stream
        .get_ref()
        .1
        .protocol_version()
        .map(|version| readable_version(&format!("{version:?}")));
    let (head, first_byte) = exchange(stream, &request).await?;
    finish(&head, dns, connect, Some(tls), first_byte, started, version)
}

fn host_header(target: &Target) -> String {
    let default = if target.https { HTTPS_PORT } else { HTTP_PORT };
    if target.port == default {
        target.host.clone()
    } else {
        format!("{}:{}", target.host, target.port)
    }
}

async fn resolve(target: &Target) -> Result<SocketAddr, WebsiteError> {
    let mut addresses = timeout(STEP_WAIT, lookup_host((target.host.as_str(), target.port)))
        .await
        .map_err(|_| WebsiteError::NoAnswer)?
        .map_err(|error| WebsiteError::Dns(error.to_string()))?;
    addresses
        .next()
        .ok_or_else(|| WebsiteError::Dns("no address".to_owned()))
}

/// `TLSv1_3` as `TLS 1.3`.
fn readable_version(debug: &str) -> String {
    debug.replacen("TLSv", "TLS ", 1).replace('_', ".")
}

/// Sends `request` and reads the answer as far as the end of its headers. Also returns how
/// long the first byte took.
async fn exchange<S>(mut stream: S, request: &str) -> Result<(Vec<u8>, Duration), WebsiteError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let sent = Instant::now();
    stream
        .write_all(request.as_bytes())
        .await
        .map_err(|error| WebsiteError::Connect(error.to_string()))?;
    let work = async {
        let mut head = Vec::new();
        let mut first_byte = None;
        let mut chunk = [0u8; 4096];
        while head.len() < MAX_HEAD_BYTES && !head.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = match stream.read(&mut chunk).await {
                Ok(0) => break,
                Ok(read) => read,
                // A server that closes without a TLS close notice has still said its piece.
                Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(error) => return Err(WebsiteError::Connect(error.to_string())),
            };
            first_byte.get_or_insert_with(|| sent.elapsed());
            head.extend_from_slice(&chunk[..read]);
        }
        Ok((head, first_byte))
    };
    let (head, first_byte) = timeout(STEP_WAIT, work)
        .await
        .map_err(|_| WebsiteError::NoAnswer)??;
    Ok((head, first_byte.ok_or(WebsiteError::NoAnswer)?))
}

fn finish(
    head: &[u8],
    dns: Duration,
    connect: Duration,
    tls: Option<Duration>,
    first_byte: Duration,
    started: Instant,
    tls_version: Option<String>,
) -> Result<Fetched, WebsiteError> {
    let (status, headers) = parse_head(head).ok_or(WebsiteError::BadReply)?;
    Ok(Fetched {
        status,
        headers,
        timings: Timings {
            dns,
            connect,
            tls,
            first_byte,
            total: started.elapsed(),
        },
        tls_version,
    })
}

/// The status and the headers of an answer. Header names are lowercase; a name that comes
/// twice keeps its first value.
fn parse_head(head: &[u8]) -> Option<(u16, Vec<(String, String)>)> {
    let text = String::from_utf8_lossy(head);
    let mut lines = text.split("\r\n");
    let status_line = lines.next()?;
    if !status_line.starts_with("HTTP/") {
        return None;
    }
    let status = status_line.split_whitespace().nth(1)?.parse().ok()?;
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in lines.take_while(|line| !line.is_empty()).take(MAX_HEADERS) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        if !headers.iter().any(|(known, _)| *known == name) {
            let value: String = value.trim().chars().filter(|c| !c.is_control()).collect();
            headers.push((name, value));
        }
    }
    Some((status, headers))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find_map(|(known, value)| (known == name).then_some(value.as_str()))
}

/// Grades the five security headers. `secure` is whether the page came over HTTPS.
fn check_headers(headers: &[(String, String)], secure: bool) -> Vec<HeaderCheck> {
    let shown = |name: &str| {
        header(headers, name).map(|value| value.chars().take(MAX_VALUE_CHARS).collect::<String>())
    };
    let csp = header(headers, "content-security-policy");
    let framing_ok = header(headers, "x-frame-options").is_some()
        || csp.is_some_and(|policy| policy.contains("frame-ancestors"));
    let hsts = header(headers, "strict-transport-security");
    let mut checks = vec![
        grade(
            "Strict-Transport-Security",
            shown("strict-transport-security"),
            match hsts {
                _ if !secure => (
                    Verdict::Weak,
                    "Only means something over HTTPS. This page came over plain HTTP.",
                ),
                None => (
                    Verdict::Weak,
                    "Missing. Browsers are not told to use HTTPS only, so a first visit can be downgraded.",
                ),
                Some(value) if max_age(value) < HSTS_MIN_SECONDS => (
                    Verdict::Weak,
                    "The max-age is short. Half a year or more keeps browsers on HTTPS.",
                ),
                Some(_) => (
                    Verdict::Good,
                    "Browsers are told to use HTTPS only for this site.",
                ),
            },
        ),
        grade(
            "Content-Security-Policy",
            shown("content-security-policy"),
            match csp {
                None => (
                    Verdict::Weak,
                    "Missing. Nothing limits what scripts and content the page may load.",
                ),
                Some(policy) if allows_unsafe_scripts(policy) => (
                    Verdict::Weak,
                    "Set, but its script rule allows unsafe-inline or unsafe-eval, which weakens it.",
                ),
                Some(_) => (Verdict::Good, "Limits what the page may load and run."),
            },
        ),
    ];
    checks.push(grade(
        "X-Content-Type-Options",
        shown("x-content-type-options"),
        match header(headers, "x-content-type-options") {
            Some(value) if value.eq_ignore_ascii_case("nosniff") => (
                Verdict::Good,
                "Browsers must trust the declared content type.",
            ),
            Some(_) => (Verdict::Weak, "Set, but not to nosniff."),
            None => (
                Verdict::Weak,
                "Missing. Browsers may guess a content type, which can run an upload as a script.",
            ),
        },
    ));
    checks.push(grade(
        "Clickjacking protection",
        shown("x-frame-options").or_else(|| {
            csp.filter(|p| p.contains("frame-ancestors"))
                .map(|_| "frame-ancestors (in Content-Security-Policy)".to_owned())
        }),
        if framing_ok {
            (
                Verdict::Good,
                "Other sites cannot show this page inside a frame.",
            )
        } else {
            (
                Verdict::Weak,
                "Missing. Another site can show this page in a frame and trick clicks.",
            )
        },
    ));
    checks.push(grade(
        "Referrer-Policy",
        shown("referrer-policy"),
        match header(headers, "referrer-policy") {
            Some("unsafe-url" | "no-referrer-when-downgrade") => {
                (Verdict::Weak, "Set, but it still sends the full address to other sites.")
            }
            Some(_) => (Verdict::Good, "Limits how much of the address other sites learn."),
            None => (
                Verdict::Weak,
                "Missing. Browsers use their own default for what address they send to other sites.",
            ),
        },
    ));
    checks
}

fn grade(
    name: &'static str,
    value: Option<String>,
    (verdict, advice): (Verdict, &str),
) -> HeaderCheck {
    HeaderCheck {
        name,
        value,
        verdict,
        advice: advice.to_owned(),
    }
}

/// Whether the rule for scripts in a Content-Security-Policy (`script-src`, or `default-src`
/// when there is none) allows `unsafe-inline` or `unsafe-eval`. The same words in a rule for
/// styles or images do not weaken what scripts may do.
fn allows_unsafe_scripts(policy: &str) -> bool {
    let rule = |name: &str| {
        policy.split(';').map(str::trim).find(|directive| {
            directive
                .split_whitespace()
                .next()
                .is_some_and(|first| first.eq_ignore_ascii_case(name))
        })
    };
    rule("script-src")
        .or_else(|| rule("default-src"))
        .is_some_and(|directive| {
            directive.contains("'unsafe-inline'") || directive.contains("'unsafe-eval'")
        })
}

/// The `max-age` of a Strict-Transport-Security value, in seconds; 0 when it has none.
fn max_age(value: &str) -> u64 {
    value
        .split(';')
        .find_map(|part| {
            let (name, number) = part.trim().split_once('=')?;
            name.trim()
                .eq_ignore_ascii_case("max-age")
                .then(|| number.trim().trim_matches('"').parse().ok())?
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use tokio::net::TcpListener;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    }

    fn verdicts(checks: &[HeaderCheck]) -> Vec<Verdict> {
        checks.iter().map(|check| check.verdict).collect()
    }

    #[test]
    fn addresses_are_read_with_a_default_scheme_port_and_path() {
        assert_eq!(
            parse_target(" example.com ").unwrap(),
            Target {
                https: true,
                host: "example.com".to_owned(),
                port: 443,
                path: "/".to_owned()
            }
        );
        let plain = parse_target("http://example.com:8080/a/b?c=1#frag").unwrap();
        assert_eq!(
            (plain.https, plain.port, plain.path.as_str()),
            (false, 8080, "/a/b?c=1")
        );
        assert_eq!(parse_target("example.com?x=1").unwrap().path, "/?x=1");
        assert_eq!(
            parse_target("https://example.com").unwrap().url(),
            "https://example.com/"
        );
        assert_eq!(plain.url(), "http://example.com:8080/a/b?c=1");
        for bad in [
            "",
            "ftp://example.com",
            "http://",
            "example.com:0",
            "exa mple.com",
            "http://-x.com",
            "example.com/a b",
        ] {
            assert_eq!(parse_target(bad), Err(WebsiteError::InvalidUrl), "{bad:?}");
        }
    }

    #[test]
    fn a_redirect_is_followed_whatever_form_the_location_takes() {
        let from = parse_target("https://example.com/a/b").unwrap();
        assert_eq!(
            follow(&from, "https://www.example.com/x").unwrap().host,
            "www.example.com"
        );
        assert_eq!(
            follow(&from, "//cdn.example.com/y").unwrap().url(),
            "https://cdn.example.com/y"
        );
        assert_eq!(
            follow(&from, "/login").unwrap().url(),
            "https://example.com/login"
        );
        assert_eq!(follow(&from, "c").unwrap().url(), "https://example.com/a/c");
        assert!(!follow(&from, "http://example.com/").unwrap().https);
        assert!(follow(&from, "javascript:alert(1)").is_err());
        assert!(follow(&from, "/a b").is_err());
    }

    #[test]
    fn headers_are_read_lowercase_and_the_first_of_a_repeat_wins() {
        let head = b"HTTP/1.1 301 Moved\r\nLocation: /x\r\nSet-Cookie: a=1\r\nset-cookie: b=2\r\nServer: nginx\x07\r\n\r\nbody";
        let (status, headers) = parse_head(head).unwrap();
        assert_eq!(status, 301);
        assert_eq!(header(&headers, "location"), Some("/x"));
        assert_eq!(header(&headers, "set-cookie"), Some("a=1"));
        assert_eq!(header(&headers, "server"), Some("nginx"));
        assert!(parse_head(b"garbage").is_none());
        assert!(parse_head(b"").is_none());
    }

    #[test]
    fn hsts_needs_a_long_enough_max_age() {
        assert_eq!(max_age("max-age=31536000; includeSubDomains"), 31_536_000);
        assert_eq!(max_age("includeSubDomains; MAX-AGE=\"600\""), 600);
        assert_eq!(max_age("includeSubDomains"), 0);
    }

    #[test]
    fn a_site_with_every_header_gets_good_across_the_board() {
        let headers = pairs(&[
            ("strict-transport-security", "max-age=63072000"),
            (
                "content-security-policy",
                "default-src 'self'; frame-ancestors 'none'",
            ),
            ("x-content-type-options", "nosniff"),
            ("referrer-policy", "strict-origin-when-cross-origin"),
        ]);
        let checks = check_headers(&headers, true);
        assert_eq!(verdicts(&checks), [Verdict::Good; 5]);
        assert_eq!(
            checks[3].value.as_deref(),
            Some("frame-ancestors (in Content-Security-Policy)")
        );
    }

    #[test]
    fn a_bare_site_gets_weak_everywhere_and_says_what_is_missing() {
        let checks = check_headers(&[], true);
        assert_eq!(verdicts(&checks), [Verdict::Weak; 5]);
        assert!(
            checks
                .iter()
                .all(|check| check.value.is_none() && check.advice.contains("Missing"))
        );
    }

    #[test]
    fn weak_settings_are_told_apart_from_missing_ones() {
        let headers = pairs(&[
            ("strict-transport-security", "max-age=300"),
            ("content-security-policy", "script-src 'unsafe-inline'"),
            ("x-content-type-options", "sniff"),
            ("x-frame-options", "DENY"),
            ("referrer-policy", "unsafe-url"),
        ]);
        let checks = check_headers(&headers, true);
        assert_eq!(
            verdicts(&checks),
            [
                Verdict::Weak,
                Verdict::Weak,
                Verdict::Weak,
                Verdict::Good,
                Verdict::Weak
            ]
        );
        assert!(checks[0].advice.contains("short") && checks[1].advice.contains("unsafe"));
    }

    #[test]
    fn unsafe_words_weaken_a_policy_only_in_the_rule_for_scripts() {
        assert!(allows_unsafe_scripts(
            "default-src 'self'; script-src 'self' 'unsafe-eval'"
        ));
        assert!(allows_unsafe_scripts("default-src 'unsafe-inline'"));
        assert!(!allows_unsafe_scripts(
            "default-src 'none'; style-src 'unsafe-inline'"
        ));
        assert!(!allows_unsafe_scripts(
            "script-src 'self'; default-src 'unsafe-inline'"
        ));
    }

    #[test]
    fn hsts_over_plain_http_is_not_expected() {
        let checks = check_headers(
            &pairs(&[("strict-transport-security", "max-age=63072000")]),
            false,
        );
        assert!(checks[0].advice.contains("plain HTTP"));
    }

    #[test]
    fn tls_versions_read_as_people_write_them() {
        assert_eq!(readable_version("TLSv1_3"), "TLS 1.3");
        assert_eq!(readable_version("TLSv1_2"), "TLS 1.2");
    }

    /// A web server that answers each connection with the next of `answers`.
    async fn web_server(answers: Vec<String>) -> u16 {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            for answer in answers {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut request = [0; 1024];
                let _ = stream.read(&mut request).await;
                let _ = stream.write_all(answer.as_bytes()).await;
            }
        });
        port
    }

    #[tokio::test]
    async fn a_redirect_chain_is_followed_and_the_last_answer_is_graded() {
        let port = web_server(vec![
            "HTTP/1.0 301 Moved\r\nLocation: /next\r\n\r\n".to_owned(),
            "HTTP/1.0 200 OK\r\nX-Content-Type-Options: nosniff\r\n\r\nhello".to_owned(),
        ])
        .await;
        let report = check(&format!("http://127.0.0.1:{port}/")).await.unwrap();
        let steps: Vec<(u16, &str)> = report
            .hops
            .iter()
            .map(|hop| (hop.status, hop.url.as_str()))
            .collect();
        assert_eq!(
            steps,
            [
                (301, format!("http://127.0.0.1:{port}/").as_str()),
                (200, format!("http://127.0.0.1:{port}/next").as_str()),
            ]
        );
        assert_eq!(report.stopped, None);
        assert!(report.timings.tls.is_none() && report.tls_version.is_none());
        assert_eq!(report.headers[2].verdict, Verdict::Good);
        assert!(report.timings.total >= report.timings.first_byte);
    }

    #[tokio::test]
    async fn a_redirect_loop_stops_at_the_limit_with_the_chain_so_far() {
        let port = web_server(vec![
            "HTTP/1.0 302 Found\r\nLocation: /\r\n\r\n".to_owned();
            20
        ])
        .await;
        let report = check(&format!("http://127.0.0.1:{port}/")).await.unwrap();
        assert_eq!(report.hops.len(), MAX_HOPS);
        assert!(report.stopped.unwrap().contains("redirects"));
    }

    #[tokio::test]
    async fn a_redirect_to_a_dead_address_keeps_what_was_learned() {
        let port = web_server(vec![
            "HTTP/1.0 302 Found\r\nLocation: http://127.0.0.1:1/\r\n\r\n".to_owned(),
        ])
        .await;
        let report = check(&format!("http://127.0.0.1:{port}/")).await.unwrap();
        assert_eq!(report.hops.len(), 1);
        assert!(report.stopped.unwrap().contains("Could not follow"));
    }

    #[tokio::test]
    async fn a_site_that_cannot_be_reached_is_an_error() {
        let error = check("http://127.0.0.1:1/").await.unwrap_err();
        assert!(matches!(error, WebsiteError::Connect(_)), "{error:?}");
        assert_eq!(
            check("ftp://x").await.unwrap_err(),
            WebsiteError::InvalidUrl
        );
    }
}
