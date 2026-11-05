//! Running a system tool and reading what it printed.

use std::io;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;

/// Runs `program` and returns whether it succeeded and everything it printed. The tool is
/// stopped after `limit`.
pub(crate) async fn run(
    program: &str,
    args: &[&str],
    limit: Duration,
) -> io::Result<(bool, String)> {
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

    #[tokio::test]
    async fn a_missing_program_is_not_found() {
        let error = run("lanping-no-such-tool", &[], Duration::from_secs(1))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
