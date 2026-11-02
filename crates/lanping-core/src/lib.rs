//! UI-free LAN scanning engine.

pub mod banner;
pub mod device_notes;
pub mod device_type;
pub mod diagnostics;
pub mod diff;
pub mod discovery;
pub mod export;
pub mod history;
pub mod host;
pub mod labels;
pub mod mdns;
pub mod neighbors;
pub mod netbios;
pub mod os_guess;
pub mod ports;
pub mod probe;
pub mod range;
pub mod scan;
pub mod ssdp;
pub mod tailnet;
pub mod timeline;
pub mod vendor;
pub mod wol;

pub use host::{Host, ServiceBanner};
pub use scan::{ScanConfig, ScanEvent, ScanHandle, ScanStage, start_scan};
