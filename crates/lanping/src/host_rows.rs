//! A host together with what the list needs to know to draw it.

use lanping_core::Host;

use crate::host_filter::RowStatus;

#[derive(Debug, Clone, Copy)]
pub struct Row<'a> {
    pub host: &'a Host,
    pub status: RowStatus,
    /// Name the user gave this device.
    pub label: Option<&'a str>,
}
