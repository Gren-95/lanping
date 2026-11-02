//! A limit on sockets open at the same time across a whole scan.
//!
//! The usual soft limit is 1024 open files per process, shared with the window
//! system. Running out does not just fail a probe: on Linux the graphics driver then
//! cannot get a window buffer, reports "out of memory", and the app ends. It also makes
//! live hosts look silent. So every socket a scan opens first takes a permit from here.

use tokio::sync::{Semaphore, SemaphorePermit};

/// Leaves room under a limit of 1024 for the window system, name lookups and files.
const MAX_OPEN_SOCKETS: usize = 384;

static BUDGET: Semaphore = Semaphore::const_new(MAX_OPEN_SOCKETS);

/// Waits for room to open a socket. Hold the permit for as long as the socket is
/// open, and do not wait for a second permit while holding one.
pub async fn socket_permit() -> Option<SemaphorePermit<'static>> {
    BUDGET.acquire().await.ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_budget_fits_well_under_the_usual_file_limit() {
        const { assert!(MAX_OPEN_SOCKETS <= 512) };
    }
}
