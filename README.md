# LanPing

Cross-platform LAN scanner written in Rust with an [Iced](https://iced.rs) GUI.

## Features

- Scan a CIDR block, an address range or a single IP (`192.168.0.0/24`, `10.0.0.1-50`)
- Runs without root or admin rights: ICMP datagram sockets, TCP connect probes and the OS ARP cache
- Hostname lookup, MAC address and vendor (IEEE OUI registry embedded)
- TCP port scan of live hosts for the ports ticked in Settings (list from Wikipedia); untick them all to skip it
- Optional service banners (SSH version, web server and similar), shown on hover and in "Open with"
- Network picker with physical interfaces and Tailscale devices; virtual adapters (Docker, VMs, VPNs) are hidden by default and can be shown in Settings
- Sortable results with filters (text, latency, status, vendor, port, name) and presets
- Customizable columns; CSV and JSON export (to the Downloads folder)
- Every finished scan is saved and can be reopened from "Saved scans"
- Rescan comparison: devices are tagged new, changed or gone against the previous scan of the same network
- Device names stored by MAC address, so labels follow a device between scans
- Wake-on-LAN and "Open with" actions: browser, SMB share, copy SSH command, copy details
- Automatic rescans with a desktop notification when a new device appears

## Build and run

```bash
cargo run --release -p lanping                                   # GUI
cargo run --release -p lanping-core --example scan_cli -- 192.168.0.0/24 22,80   # CLI
cargo test --workspace
scripts/build-appimage.sh                                          # needs appimagetool on PATH
```

## Layout

- `crates/lanping-core`: scan engine, no UI dependencies
- `crates/lanping`: Iced application

## Notes

- IPv4 only for now.
- Settings, device names and saved scans live in the platform config and data folders (`lanping`).
- Notifications use `notify-send` (Linux) and `osascript` (macOS); Windows shows results in the app only.
- Windows and macOS builds type-check for Windows; they have not been run there.
- Hosts that ignore ping and TCP are found through the ARP cache and show no latency. Entries
  the OS cached shortly before the scan can appear as hosts that have just left the network.
- `crates/lanping-core/data/oui.tsv` is generated from the IEEE MA-L registry
  (`https://standards-oui.ieee.org/oui/oui.csv`).
- Common port numbers and names follow Wikipedia's
  [List of TCP and UDP port numbers](https://en.wikipedia.org/wiki/List_of_TCP_and_UDP_port_numbers) (CC BY-SA).
- Icons come from [Lucide](https://lucide.dev) (ISC license) through the `iced_fonts` crate.
