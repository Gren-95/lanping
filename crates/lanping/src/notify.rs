//! Desktop notifications through the platform's own command line tool.

use std::io;
#[cfg(any(target_os = "linux", target_os = "macos"))]
use std::process::Command;

#[cfg(target_os = "linux")]
const APP_NAME: &str = "LanPing";

/// Removes control characters: device names come from the network and must
/// not be able to inject extra lines or terminal escapes.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn sanitize(text: &str) -> String {
    text.chars().filter(|character| !character.is_control()).collect()
}

#[cfg(target_os = "macos")]
fn escape_applescript(text: &str) -> String {
    text.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(target_os = "linux")]
pub fn send(title: &str, body: &str) -> io::Result<()> {
    let mut child = Command::new("notify-send")
        .args(["--app-name", APP_NAME, "--"])
        .arg(sanitize(title))
        .arg(sanitize(body))
        .spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn send(title: &str, body: &str) -> io::Result<()> {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        escape_applescript(&sanitize(body)),
        escape_applescript(&sanitize(title))
    );
    let mut child = Command::new("osascript").args(["-e", &script]).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub fn send(_title: &str, _body: &str) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "desktop notifications are not supported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn strips_control_characters() {
        assert_eq!(sanitize("evil\nname\x1b[31m"), "evilname[31m");
    }
}
