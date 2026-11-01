//! Ping and traceroute through the system's own tools, so the output is the
//! familiar one and no raw-socket privileges are needed.

use std::fmt;
use std::io;
use std::net::Ipv4Addr;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

/// Longest a diagnostic may run before it is stopped.
const RUN_LIMIT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Diagnostic {
    Ping,
    Traceroute,
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Ping => "Ping",
            Self::Traceroute => "Traceroute",
        })
    }
}

/// A program and its arguments.
type CommandLine = (&'static str, Vec<String>);

impl Diagnostic {
    /// Commands to try in order; the first one installed on this system is used.
    fn candidates(self, ip: Ipv4Addr) -> Vec<CommandLine> {
        let target = ip.to_string();
        let args = |flags: &[&str]| -> Vec<String> {
            flags
                .iter()
                .map(|flag| (*flag).to_owned())
                .chain(std::iter::once(target.clone()))
                .collect()
        };
        match (self, cfg!(windows)) {
            (Self::Ping, true) => vec![("ping", args(&["-n", "4"]))],
            (Self::Ping, false) => vec![("ping", args(&["-c", "4"]))],
            (Self::Traceroute, true) => vec![("tracert", args(&["-d", "-h", "20"]))],
            (Self::Traceroute, false) => vec![
                ("traceroute", args(&["-n", "-q", "1", "-m", "20"])),
                ("tracepath", args(&["-n"])),
            ],
        }
    }
}

/// Runs the diagnostic against `ip` and returns what the tool printed.
/// A non-zero exit still returns the output, since a failed ping says why.
pub async fn run(diagnostic: Diagnostic, ip: Ipv4Addr) -> Result<String, String> {
    let candidates = diagnostic.candidates(ip);
    for (program, args) in &candidates {
        match run_command(program, args).await {
            Ok(output) => return Ok(output),
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(format!("Could not run {program}: {error}")),
        }
    }
    let names: Vec<&str> = candidates.iter().map(|(program, _)| *program).collect();
    Err(format!(
        "{} is not installed on this system",
        names.join(" or ")
    ))
}

async fn run_command(program: &str, args: &[String]) -> io::Result<String> {
    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()?;
    let finished = timeout(RUN_LIMIT, child.wait_with_output())
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "timed out after 60 seconds"))??;
    let mut text = String::from_utf8_lossy(&finished.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&finished.stderr));
    Ok(text.trim_end().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_commands_that_end_with_the_target_address() {
        let ip = Ipv4Addr::new(192, 168, 0, 9);
        for diagnostic in [Diagnostic::Ping, Diagnostic::Traceroute] {
            let candidates = diagnostic.candidates(ip);
            assert!(!candidates.is_empty());
            for (_, args) in candidates {
                assert_eq!(args.last().map(String::as_str), Some("192.168.0.9"));
            }
        }
    }

    #[test]
    fn titles_name_the_tool() {
        assert_eq!(Diagnostic::Ping.to_string(), "Ping");
        assert_eq!(Diagnostic::Traceroute.to_string(), "Traceroute");
    }

    #[tokio::test]
    async fn pings_the_loopback_address_when_ping_is_installed() {
        match run(Diagnostic::Ping, Ipv4Addr::LOCALHOST).await {
            Ok(output) => assert!(output.contains("127.0.0.1"), "unexpected output: {output}"),
            Err(error) => assert!(error.contains("not installed"), "unexpected error: {error}"),
        }
    }

    #[tokio::test]
    async fn a_missing_program_is_reported_as_not_found() {
        let error = run_command("lanping-no-such-tool", &[]).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
