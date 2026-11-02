//! Finds the default gateway from the operating system's routing table.

use std::net::Ipv4Addr;
use std::process::Command;

/// The address after `via` in `ip -4 route show default` output.
fn parse_ip_route(text: &str) -> Option<Ipv4Addr> {
    text.lines().find_map(|line| {
        let mut tokens = line.split_whitespace();
        tokens.by_ref().find(|token| *token == "via")?;
        tokens.next()?.parse().ok()
    })
}

/// The `gateway:` line of macOS `route -n get default` output.
fn parse_route_get(text: &str) -> Option<Ipv4Addr> {
    text.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        (name.trim() == "gateway").then(|| value.trim().parse().ok())?
    })
}

/// The `0.0.0.0  0.0.0.0  <gateway>` row of Windows `route print` output.
fn parse_route_print(text: &str) -> Option<Ipv4Addr> {
    text.lines().find_map(|line| {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        match tokens.as_slice() {
            ["0.0.0.0", "0.0.0.0", gateway, ..] => gateway.parse().ok(),
            _ => None,
        }
    })
}

/// The default gateway, or `None` when there is none or the tool is missing.
pub fn default_gateway() -> Option<Ipv4Addr> {
    let (program, args, parse): (&str, &[&str], fn(&str) -> Option<Ipv4Addr>) = if cfg!(windows) {
        ("route", &["print", "-4", "0.0.0.0"], parse_route_print)
    } else if cfg!(target_os = "macos") {
        ("route", &["-n", "get", "default"], parse_route_get)
    } else {
        ("ip", &["-4", "route", "show", "default"], parse_ip_route)
    };
    let output = Command::new(program).args(args).output().ok()?;
    parse(&String::from_utf8_lossy(&output.stdout))
}

#[cfg(test)]
mod tests {
    use super::*;

    const GATEWAY: Ipv4Addr = Ipv4Addr::new(192, 168, 1, 1);

    #[test]
    fn reads_linux_routes() {
        let text = "default via 192.168.1.1 dev wlan0 proto dhcp metric 600\n";
        assert_eq!(parse_ip_route(text), Some(GATEWAY));
        assert_eq!(parse_ip_route("default dev tun0 scope link\n"), None);
        assert_eq!(parse_ip_route(""), None);
    }

    #[test]
    fn reads_macos_routes() {
        let text = "   route to: default\n    gateway: 192.168.1.1\n  interface: en0\n";
        assert_eq!(parse_route_get(text), Some(GATEWAY));
        assert_eq!(parse_route_get("    gateway: link#5\n"), None);
    }

    #[test]
    fn reads_windows_routes() {
        let text = "Network Destination   Netmask    Gateway   Interface  Metric\n\
                    0.0.0.0   0.0.0.0   192.168.1.1   192.168.1.20   35\n";
        assert_eq!(parse_route_print(text), Some(GATEWAY));
        assert_eq!(parse_route_print("127.0.0.0 255.0.0.0 On-link\n"), None);
    }
}
