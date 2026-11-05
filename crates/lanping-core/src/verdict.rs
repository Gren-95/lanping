//! How good a result is, in the three grades the checks use.

/// Good is as it should be. Weak works but could be better. Missing is not there at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Good,
    Weak,
    Missing,
}
