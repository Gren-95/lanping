//! Hints about open ports worth a second look. A port scan only shows that
//! something answers, so these are prompts to check, not findings.

/// Why `port` is worth a look, or `None` when there is nothing to say.
pub fn port_hint(port: u16) -> Option<&'static str> {
    Some(match port {
        21 => "FTP sends passwords unencrypted",
        23 | 2323 => "Telnet sends everything unencrypted",
        69 => "TFTP has no login at all",
        135 | 139 => "Windows networking; usually not meant to be widely reachable",
        445 => "File sharing; check that the old SMBv1 protocol is off",
        512..=514 => "Old remote login services without encryption",
        1433 | 3306 | 5432 | 27017 | 6379 => "A database; usually not meant to be widely reachable",
        1723 => "PPTP VPN uses weak, outdated encryption",
        3389 => "Remote desktop; a common target, keep it off untrusted networks",
        5900 => "VNC remote control; often weakly protected",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hints_at_known_risky_services() {
        assert!(port_hint(23).unwrap().contains("Telnet"));
        assert!(port_hint(445).unwrap().contains("SMBv1"));
        assert!(port_hint(3389).is_some());
    }

    #[test]
    fn says_nothing_about_ordinary_ports() {
        assert_eq!(port_hint(22), None);
        assert_eq!(port_hint(443), None);
        assert_eq!(port_hint(8080), None);
    }
}
