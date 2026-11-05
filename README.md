<div align="center">

<img src="packaging/lanping.svg" alt="" width="120">

# LanPing

**See every device on your network.**

A LAN scanner for the desktop, written in Rust with an [Iced](https://iced.rs) GUI. No root or admin rights needed.

[![CI](https://github.com/Gren-95/lanping/actions/workflows/ci.yml/badge.svg)](https://github.com/Gren-95/lanping/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Gren-95/lanping)](https://github.com/Gren-95/lanping/releases/latest)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![Rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange)](https://www.rust-lang.org)

[**Download**](https://github.com/Gren-95/lanping/releases/latest)

<img src="docs/screenshot.png" alt="LanPing showing a made-up network" width="800">

<sub>The screenshot uses the made-up scans in <code>docs/demo</code>.</sub>

</div>

## Download

Get the latest build from [Releases](https://github.com/Gren-95/lanping/releases/latest):

- **Linux:** `LanPing-<version>-x86_64.AppImage`. Make it executable with `chmod +x`, then run it.
- **Windows:** `LanPing-<version>-x86_64.exe`. It is unsigned, so SmartScreen warns on first launch.

- **macOS:** `LanPing-<version>-macos-universal.dmg`, for Intel and Apple silicon.

> [!WARNING]
> **The macOS build is untested.** It is built automatically but has never been run on a Mac, so it may not work. It is also not notarized, so macOS blocks the first launch: drag it to Applications, then right-click it and choose Open, or allow it under System Settings > Privacy & Security. Allow local network access when asked, or scans find nothing. Please report any problems in the [issues](https://github.com/Gren-95/lanping/issues).

## Features

**Scanning**

- Scan a CIDR block, an address range or a single IP (`192.168.0.0/24`, `10.0.0.1-50`)
- Runs without root or admin rights: ICMP datagram sockets, TCP connect probes and the OS ARP cache
- Hostname, MAC address and vendor (IEEE OUI registry embedded), plus IPv6 addresses from the neighbor cache
- Names, models and services that devices announce over mDNS, NetBIOS and SSDP/UPnP
- Optional read-only SNMP v2c for model, serial number, uptime and port count
- TCP port scan of live hosts for the ports ticked in Settings; untick them all to skip it
- Optional service banners, such as the SSH version or web server
- Suggestions for your networks and Tailscale devices as you type; virtual adapters (Docker, VMs, VPNs) are hidden by default
- Optional scans at startup, on a timer or when the main network changes, with desktop notifications for new devices, newly opened ports and named devices going offline

**Results**

- Sortable table or card view, with search, filters and saved presets
- A best guess at each device's type, with an icon and a filter, and at its operating system
- Reorder and hide columns by dragging, from Settings or by right-clicking a column header
- Click a device for its ports, banners, scan history and a timeline of what changed
- Devices are tagged new, changed or gone against the previous scan of the same network
- Open ports worth a second look are shown in amber, with a hint (not a security audit)
- Name your devices and note the owner, location and tags; these are stored by MAC address, so they follow a device between scans
- Every finished scan is saved and can be reopened from "Saved scans"
- A summary of totals by type, vendor and open port, and a map of devices grouped around the gateway
- Export to CSV, JSON or a printable HTML report (a browser can save it as a PDF), or copy the visible rows

**Wi-Fi**

- A Wi-Fi tab lists nearby access points with channel, band, signal and security, and shows how crowded each channel is with a suggested channel for your router
- Sort, search and filter the list, copy names and BSSIDs, and turn on auto refresh to watch each network's signal trend over the last 40 scans
- Uses `nmcli` on Linux and `netsh` on Windows, so no root is needed; not available on macOS yet

**Trace**

- A Trace tab repeats a traceroute to any host name or IP address and shows loss, latency, jitter and a latency graph for every hop, like MTR
- Runs the system `traceroute` (or `tracepath`, `tracert` on Windows) once per round, so no root is needed and nothing extra is installed

**Latency**

- A Latency tab probes one host every second, 5 seconds or 30 seconds with ICMP ping (TCP connects when the system does not allow unprivileged ping), shows loss, average, best, worst and jitter with a history graph, and saves the log as CSV

**Speed**

- A Speed tab measures download and upload speed against `speed.cloudflare.com` or against M-Lab's open NDT7 servers around the world (pick the nearest, or a country, and a server), and between two computers that run LanPing: start the test server on one (default port 5252) and run a test against it from the other. A test measures ping first, then download and upload (or only one of them), and shows the speed of each second as a graph. For M-Lab, pick the nearest servers or one of the countries M-Lab actually has a server in (the list comes from M-Lab itself; there is none in Estonia, so use "nearest" there)
- Finished speed tests and latency runs are saved next to the saved scans and listed on the History page

**DNS**

- A DNS tab looks up A, AAAA, MX, TXT, NS, PTR and CNAME records at the resolvers your system uses or at Cloudflare, Google and Quad9, and times all of them side by side. Queries are sent directly over UDP (TCP for long answers), so nothing extra is installed
- A WHOIS page in the same tab shows who is behind a domain name or an IP address: registrar, dates and name servers, or network name, organization and range, above the full answer

**TLS**

- A TLS tab reads the certificate chain a server presents on any host and port: subject, issuer, validity dates, days left and the names it is valid for, and says whether this system would trust it. Expired, self-signed and mismatched certificates are shown instead of failing, an optional server name lets you check an IP address on a server that hosts several sites, such as Caddy, and an open HTTPS port in the device menu offers a one-click check

**SNMP**

- An SNMP tab asks a router or switch over read-only SNMP v2c how much each interface carries: status, link speed, live in and out rates, how full the link is, and a graph. It uses the 64-bit counters where the device has them and copes with counter wraps and device restarts. The community is typed by you and never guessed. SNMP counts per interface, not per device on the network. `docker/snmp-test` has a throwaway SNMP device in a container to try it against

**Subnet**

- A Subnet tab turns an address with a prefix length or netmask into network, broadcast, host range and host count, for IPv4 and IPv6, and can start a scan of the range

**Actions**

- Open in a browser or browse an SMB share
- Ping, traceroute and Wake-on-LAN
- Copy the SSH command, IP, MAC or hostname
- Choose which tabs show, and in what order, in Settings under Tabs (Devices always stays). TLS, SNMP and Subnet start hidden
- Keyboard shortcuts: Ctrl+F to search, Enter to scan, Esc to clear filters or close a dialog

## Build and run

Needs Rust 1.90 or newer.

```bash
cargo run --release -p lanping                                   # GUI
cargo run --release -p lanping-core --example scan_cli -- 192.168.0.0/24 22,80   # CLI
cargo test --workspace
scripts/build-appimage.sh                                          # needs appimagetool on PATH
```

## Demo data

`docs/demo` holds two made-up scans, so the app can be shown without a real network. To open them without touching your own settings or saved scans:

```bash
demo="$(mktemp -d)"
mkdir -p "$demo/data/lanping/scans"
cp docs/demo/*.json "$demo/data/lanping/scans/"
XDG_CONFIG_HOME="$demo/config" XDG_DATA_HOME="$demo/data" \
  cargo run --release -p lanping -- "$demo/data/lanping/scans/scan-1790003600.json"
```

Any saved scan file can be opened the same way: `lanping path/to/scan.json`.

## Releases

Release builds never run on their own. Push a version tag, then start the workflow on it:

```sh
git tag v0.2.0 && git push origin v0.2.0
gh workflow run release.yml --ref v0.2.0
```

This builds the Linux AppImage, the Windows `.exe` and the macOS `.dmg` in GitHub Actions and attaches them to a GitHub Release. Started on a branch instead, it only builds them and keeps them as workflow artifacts.

## Layout

- `crates/lanping-core`: scan engine, no UI dependencies
- `crates/lanping`: Iced application

## Notes

- Scanning is IPv4 only. Found devices also show the IPv6 addresses in the OS neighbor cache (Linux and macOS), matched by MAC address.
- Settings, device names and saved scans live in the platform config and data folders (`lanping`). Data from the earlier `lansweep` folders is moved there on first start.
- Notifications use `notify-send` (Linux) and `osascript` (macOS); Windows shows results in the app only.
- Hosts that ignore ping and TCP are found through the ARP cache and show no latency. Entries
  the OS cached shortly before the scan can appear as hosts that have just left the network.
- `crates/lanping-core/data/oui.tsv` is generated from the IEEE MA-L registry
  (`https://standards-oui.ieee.org/oui/oui.csv`).
- Common port numbers and names follow Wikipedia's
  [List of TCP and UDP port numbers](https://en.wikipedia.org/wiki/List_of_TCP_and_UDP_port_numbers) (CC BY-SA).
- Icons come from [Lucide](https://lucide.dev) (ISC license) through the `iced_fonts` crate.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option.
