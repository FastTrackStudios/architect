//! A counting global allocator for a **test binary**: live heap bytes and
//! their high-water mark, so a test can hold a server to a memory budget
//! in bytes the *program* asked for (the number a regression moves),
//! rather than RSS, which also counts what the allocator kept, the
//! binary's own pages and every other test's leftovers.
//!
//! architect never installs it. It is a type, not a `#[global_allocator]`:
//! a library that installed one would impose it on every process that
//! links the library. The test binary that wants the counters opts in with
//! one line in its crate root:
//!
//! ```ignore
//! #[global_allocator]
//! static ALLOC: architect::testing::alloc::Counting = architect::testing::alloc::Counting;
//! ```
//!
//! Without that line the counters stay at zero, which
//! `crates/architect/tests/testing_alloc_uninstalled.rs` pins.
//!
//! The cost is two relaxed atomics per allocation. The counters are
//! process-wide: read them only when the test has the process to itself
//! ([`counters_are_this_tests`]), which is how nextest runs every test.
//!
//! Extracted from Task's `apps/server/tests/it/support/alloc.rs` (STA-400).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

/// [`System`], counted. Install it with `#[global_allocator]` in a test
/// binary (see the module docs); never in a library.
#[derive(Debug, Default, Clone, Copy)]
pub struct Counting;

fn grew(by: usize) {
    let before = LIVE.fetch_add(by, Ordering::Relaxed);
    PEAK.fetch_max(before.wrapping_add(by), Ordering::Relaxed);
}

fn shrank(by: usize) {
    LIVE.fetch_sub(by, Ordering::Relaxed);
}

// SAFETY: every method forwards to `System` unchanged; the counters are
// bookkeeping beside it and never touch the memory handed out.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded verbatim; the caller upholds `alloc`'s contract.
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: forwarded verbatim.
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: forwarded verbatim.
        unsafe { System.dealloc(ptr, layout) };
        shrank(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: forwarded verbatim.
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            match new_size.checked_sub(layout.size()) {
                Some(grown) => grew(grown),
                None => shrank(layout.size().saturating_sub(new_size)),
            }
        }
        p
    }
}

/// The variable nextest sets in every test process it starts.
pub const NEXTEST_ENV: &str = "NEXTEST";

/// Whether this test has the process, and so the counters, to itself.
///
/// nextest runs every test in a process of its own and sets `NEXTEST`;
/// plain `cargo test` runs a binary's tests on threads of one process,
/// where every other test's allocations land in the same counters. A
/// budget test should skip itself there (this prints a line saying so)
/// rather than fail at random.
#[must_use]
pub fn counters_are_this_tests() -> bool {
    if std::env::var_os(NEXTEST_ENV).is_some() {
        return true;
    }
    eprintln!(
        "skipped: the heap counters are process-wide and other tests share this process; \
         run it under `cargo nextest run`"
    );
    false
}

/// Heap bytes live right now. Zero when [`Counting`] is not installed.
#[must_use]
pub fn live() -> usize {
    LIVE.load(Ordering::Relaxed)
}

/// The most heap bytes live at once since the last [`reset_peak`] (or
/// since the process started).
#[must_use]
pub fn peak() -> usize {
    PEAK.load(Ordering::Relaxed)
}

/// Start a new high-water mark from what is live now, so the peak of one
/// call reads as `peak() - live_before`.
pub fn reset_peak() {
    PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed);
}

/// This process's anonymous resident memory in KiB (`RssAnon` in
/// `/proc/self/status`): the part of RSS that is heap, stacks and
/// anonymous maps rather than the binary's pages. `None` off Linux.
#[must_use]
pub fn rss_anon_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|l| l.strip_prefix("RssAnon:"))
        .and_then(|v| v.trim().trim_end_matches("kB").trim().parse().ok())
}

/// `bytes` as MiB with one decimal (`"1.5 MiB"`), for a report line.
/// Truncates rather than rounds the decimal.
#[must_use]
pub fn mib(bytes: usize) -> String {
    const MIB: usize = 1 << 20;
    let whole = bytes.checked_div(MIB).unwrap_or_default();
    let tenths = bytes
        .checked_rem(MIB)
        .unwrap_or_default()
        .saturating_mul(10)
        .checked_div(MIB)
        .unwrap_or_default();
    format!("{whole}.{tenths} MiB")
}

#[cfg(test)]
mod tests {
    use super::mib;

    #[test]
    fn mib_formats_one_decimal() {
        assert_eq!(mib(0), "0.0 MiB");
        assert_eq!(mib(1 << 20), "1.0 MiB");
        assert_eq!(mib((1 << 20) + (1 << 19)), "1.5 MiB");
        assert_eq!(mib(10 * (1 << 20) - 1), "9.9 MiB");
    }
}
