//! UI-free LAN scanning engine.

pub mod banner;
pub mod diagnostics;
pub mod diff;
pub mod export;
pub mod history;
pub mod host;
pub mod labels;
pub mod neighbors;
pub mod ports;
pub mod probe;
pub mod range;
pub mod scan;
pub mod tailnet;
pub mod vendor;
pub mod wol;

pub use host::{Host, ServiceBanner};
pub use scan::{ScanConfig, ScanEvent, ScanHandle, ScanStage, start_scan};
