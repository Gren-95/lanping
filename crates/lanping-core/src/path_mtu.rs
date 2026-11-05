//! The largest packet that reaches a host without being split: the path MTU.
//!
//! A wrong MTU explains connections that stall or crawl, for example over a VPN or a PPPoE
//! link. On Linux the system `tracepath` is asked first, since it also says where the size
//! dropped. Everywhere else, and when `tracepath` is missing, `ping` with the
//! do-not-fragment flag is run with sizes found by a search. No raw sockets are used.

use std::fmt;
use std::future::Future;
use std::io;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

use crate::trace::{TraceError, validate_target};

/// The usual MTUs, for comparison with what was found.
pub const USUAL_MTUS: [(u32, &str); 5] = [
    (1500, "Ethernet"),
    (1492, "PPPoE (many DSL links)"),
    (1480, "IPv6 or 6in4 tunnel"),
    (1420, "WireGuard"),
    (1400, "many VPNs"),
];

/// IPv4 and ICMP headers added to the payload a ping carries.
const HEADER_BYTES: u32 = 28;
/// The least every IPv4 host must accept.
const MIN_MTU: u32 = 576;
const MAX_MTU: u32 = 1500;
const PING_WAIT: Duration = Duration::from_secs(2);
const TRACEPATH_LIMIT: Duration = Duration::from_secs(25);
/// Hops `tracepath` is asked to try, so a path that goes quiet gives up sooner.
const TRACEPATH_HOPS: &str = "15";
/// The word before the size in `tracepath` output.
const PMTU_WORD: &str = "pmtu";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Tracepath,
    PingSearch,
}

/// The hop where the size dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub number: u32,
    pub address: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MtuReport {
    pub target: String,
    pub mtu: u32,
    pub method: Method,
    /// Where the path got narrower than the first hop. Only `tracepath` says.
    pub narrowed_at: Option<Hop>,
    /// A full-size packet got through, and larger sizes were not tried.
    pub at_least: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MtuError {
    InvalidTarget,
    /// Nothing answered, not even the smallest packet.
    NoAnswer,
    /// Neither `tracepath` nor `ping` could be run.
    NoTool,
    Failed(String),
}

impl fmt::Display for MtuError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidTarget => f.write_str("Enter a host name or an IP address"),
            Self::NoAnswer => f.write_str(
                "The host did not answer, so no size could be measured. Check the address \
                 and that it answers ping.",
            ),
            Self::NoTool => f.write_str("Neither tracepath nor ping is installed on this system"),
            Self::Failed(error) => write!(f, "Could not measure the path: {error}"),
        }
    }
}

impl From<TraceError> for MtuError {
    fn from(_: TraceError) -> Self {
        Self::InvalidTarget
    }
}

/// What a value is usually the sign of, when it is one of the usual ones.
pub fn usual_name(mtu: u32) -> Option<&'static str> {
    USUAL_MTUS
        .iter()
        .find_map(|(usual, name)| (*usual == mtu).then_some(*name))
}

/// Measures the path to `target`, a host name or IP address.
pub async fn check(target: &str) -> Result<MtuReport, MtuError> {
    let target = validate_target(target)?;
    if cfg!(target_os = "linux")
        && let Some(report) = via_tracepath(target).await
    {
        return Ok(report);
    }
    via_ping(target).await
}

async fn via_tracepath(target: &str) -> Option<MtuReport> {
    let (_, output) = run(
        "tracepath",
        &["-n", "-m", TRACEPATH_HOPS, target],
        TRACEPATH_LIMIT,
    )
    .await
    .ok()?;
    let (mtu, narrowed_at) = parse_tracepath(&output)?;
    Some(MtuReport {
        target: target.to_owned(),
        mtu,
        method: Method::Tracepath,
        narrowed_at,
        at_least: false,
    })
}

/// The path MTU in `tracepath -n` output, and the first hop where it got smaller. Lines
/// look like ` 2:  10.10.0.1   5.2ms pmtu 1492`, and the last one `Resume: pmtu 1492 hops 5`.
fn parse_tracepath(output: &str) -> Option<(u32, Option<Hop>)> {
    let mut current: Option<u32> = None;
    let mut narrowed_at = None;
    let mut resume = None;
    for line in output.lines() {
        let Some(mtu) = pmtu_in(line) else {
            continue;
        };
        if line.trim_start().starts_with("Resume:") {
            resume = Some(mtu);
            continue;
        }
        if narrowed_at.is_none() && current.is_some_and(|before| mtu < before) {
            narrowed_at = hop_of(line);
        }
        current = Some(mtu);
    }
    resume.or(current).map(|mtu| (mtu, narrowed_at))
}

fn pmtu_in(line: &str) -> Option<u32> {
    let mut words = line.split_whitespace();
    words.find(|word| *word == PMTU_WORD)?;
    words.next()?.parse().ok()
}

fn hop_of(line: &str) -> Option<Hop> {
    let mut words = line.split_whitespace();
    let number = words
        .next()?
        .trim_end_matches(':')
        .trim_end_matches('?')
        .parse()
        .ok()?;
    let address = words
        .next()
        .filter(|word| *word != "no" && !word.starts_with('['))
        .map(str::to_owned);
    Some(Hop { number, address })
}

async fn via_ping(target: &str) -> Result<MtuReport, MtuError> {
    let fits = |mtu: u32| {
        let target = target.to_owned();
        async move { ping_fits(&target, mtu).await }
    };
    let (mtu, at_least) = largest_fitting(MIN_MTU, MAX_MTU, fits).await?;
    Ok(MtuReport {
        target: target.to_owned(),
        mtu,
        method: Method::PingSearch,
        narrowed_at: None,
        at_least,
    })
}

/// The largest size from `low` to `high` that `fits`, found by halving, and whether `high`
/// itself fitted. A path is taken to be as wide at every size below one that fitted.
async fn largest_fitting<F, Fut>(low: u32, high: u32, mut fits: F) -> Result<(u32, bool), MtuError>
where
    F: FnMut(u32) -> Fut,
    Fut: Future<Output = Result<bool, MtuError>>,
{
    if fits(high).await? {
        return Ok((high, true));
    }
    if !fits(low).await? {
        return Err(MtuError::NoAnswer);
    }
    let (mut works, mut fails) = (low, high);
    while fails - works > 1 {
        let middle = works + (fails - works) / 2;
        if fits(middle).await? {
            works = middle;
        } else {
            fails = middle;
        }
    }
    Ok((works, false))
}

/// Whether a packet of `mtu` bytes gets to `target` in one piece.
async fn ping_fits(target: &str, mtu: u32) -> Result<bool, MtuError> {
    let payload = (mtu - HEADER_BYTES).to_string();
    let wait = PING_WAIT.as_secs().to_string();
    let args: Vec<&str> = if cfg!(windows) {
        vec!["-n", "1", "-f", "-l", &payload, "-w", "2000", target]
    } else if cfg!(target_os = "macos") {
        vec!["-c", "1", "-D", "-s", &payload, "-t", &wait, target]
    } else {
        vec!["-c", "1", "-W", &wait, "-M", "do", "-s", &payload, target]
    };
    let (success, output) = run("ping", &args, PING_WAIT * 3)
        .await
        .map_err(|error| match error.kind() {
            io::ErrorKind::NotFound => MtuError::NoTool,
            _ => MtuError::Failed(error.to_string()),
        })?;
    Ok(answered(success, &output))
}

/// Windows `ping` exits well even when a router says the packet is too big, so there the
/// answer from the target itself (a `TTL=` field) is what counts.
fn answered(success: bool, output: &str) -> bool {
    if cfg!(windows) {
        output.contains("TTL=")
    } else {
        success
    }
}

async fn run(program: &str, args: &[&str], limit: Duration) -> io::Result<(bool, String)> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let finished = timeout(limit, child.wait_with_output())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timed out"))??;
    let mut text = String::from_utf8_lossy(&finished.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&finished.stderr));
    Ok((finished.status.success(), text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const NARROWED: &str = " 1?: [LOCALHOST]                      pmtu 1500\n \
        1:  192.168.0.1                                           0.523ms\n \
        1:  192.168.0.1                                           0.388ms\n \
        2:  10.10.0.1                                             5.2ms pmtu 1492\n \
        2:  10.10.0.1                                             5.0ms\n \
        3:  no reply\n \
        4:  93.184.216.34                                        20.1ms reached\n     \
        Resume: pmtu 1492 hops 4 back 4";

    #[test]
    fn tracepath_output_gives_the_mtu_and_the_hop_where_it_dropped() {
        let (mtu, hop) = parse_tracepath(NARROWED).unwrap();
        assert_eq!(mtu, 1492);
        assert_eq!(
            hop,
            Some(Hop {
                number: 2,
                address: Some("10.10.0.1".to_owned())
            })
        );
    }

    #[test]
    fn a_path_that_never_narrows_has_no_such_hop() {
        let output = " 1?: [LOCALHOST]   pmtu 1500\n 1:  192.168.0.1   0.5ms\n \
             2:  8.8.8.8   9ms reached\n     Resume: pmtu 1500 hops 2 back 2";
        assert_eq!(parse_tracepath(output), Some((1500, None)));
    }

    #[test]
    fn output_without_a_size_gives_nothing() {
        assert_eq!(parse_tracepath(""), None);
        assert_eq!(
            parse_tracepath("tracepath: host: Name or service not known"),
            None
        );
    }

    #[test]
    fn a_hop_with_no_address_still_has_its_number() {
        let hop = hop_of(" 3:  no reply   pmtu 1400").unwrap();
        assert_eq!((hop.number, hop.address), (3, None));
    }

    async fn search(limit: u32) -> Result<(u32, bool), MtuError> {
        largest_fitting(MIN_MTU, MAX_MTU, |mtu| async move { Ok(mtu <= limit) }).await
    }

    #[tokio::test]
    async fn the_search_finds_the_largest_size_that_fits() {
        assert_eq!(search(1492).await, Ok((1492, false)));
        assert_eq!(search(1420).await, Ok((1420, false)));
        assert_eq!(search(576).await, Ok((576, false)));
        assert_eq!(search(1499).await, Ok((1499, false)));
    }

    #[tokio::test]
    async fn a_full_size_packet_that_fits_ends_the_search_at_once() {
        assert_eq!(search(9000).await, Ok((1500, true)));
    }

    #[tokio::test]
    async fn a_path_that_takes_nothing_is_no_answer() {
        assert_eq!(search(100).await, Err(MtuError::NoAnswer));
    }

    #[tokio::test]
    async fn the_search_asks_only_a_few_times() {
        let asked = std::cell::Cell::new(0);
        let _ = largest_fitting(MIN_MTU, MAX_MTU, |mtu| {
            asked.set(asked.get() + 1);
            async move { Ok(mtu <= 1280) }
        })
        .await;
        assert!(asked.get() <= 12, "{}", asked.get());
    }

    #[test]
    fn usual_values_are_named() {
        assert_eq!(usual_name(1492), Some("PPPoE (many DSL links)"));
        assert_eq!(usual_name(1493), None);
    }

    #[tokio::test]
    async fn a_bad_target_is_refused_before_anything_runs() {
        assert_eq!(check("-c 5").await, Err(MtuError::InvalidTarget));
        assert_eq!(check("").await, Err(MtuError::InvalidTarget));
    }
}
