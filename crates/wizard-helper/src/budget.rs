//! Test support. `PROPTEST_CASES` (CI sets 20000) overrides a property's own
//! `with_cases`, so a property that touches the disk counts its own runs and
//! stops after `n` of them.

use std::sync::atomic::{AtomicU32, Ordering};

/// True for the first `n` calls that share `runs`.
pub(crate) fn within(runs: &AtomicU32, n: u32) -> bool {
    runs.fetch_add(1, Ordering::Relaxed) < n
}
