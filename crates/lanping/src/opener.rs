//! Hands a URL or network share to the desktop's default handler.

use std::io;
use std::net::Ipv4Addr;
use std::process::Command;

#[cfg(target_os = "windows")]
const OPEN_PROGRAM: &str = "explorer";
#[cfg(target_os = "macos")]
const OPEN_PROGRAM: &str = "open";
#[cfg(all(unix, not(target_os = "macos")))]
const OPEN_PROGRAM: &str = "xdg-open";

pub fn open_target(target: &str) -> io::Result<()> {
    let mut child = Command::new(OPEN_PROGRAM).arg(target).spawn()?;
    // Reap the launcher so it does not linger as a zombie process.
    std::thread::spawn(move || child.wait());
    Ok(())
}

/// Network share address in the form the platform file manager expects.
pub fn share_target(ip: Ipv4Addr) -> String {
    if cfg!(target_os = "windows") {
        format!(r"\\{ip}")
    } else {
        format!("smb://{ip}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_share_target_for_platform() {
        let target = share_target(Ipv4Addr::new(10, 0, 0, 7));
        if cfg!(target_os = "windows") {
            assert_eq!(target, r"\\10.0.0.7");
        } else {
            assert_eq!(target, "smb://10.0.0.7");
        }
    }
}
