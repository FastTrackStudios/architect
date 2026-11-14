//! What the memory measurements share, natively (`tests/memory.rs`) and
//! in a browser's engine (`examples/index_memory.rs`, run under Node by
//! `examples/index_memory.mjs`): a generated vault, its index's snapshot,
//! and a counting allocator's numbers.

#![allow(dead_code, clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicIsize, Ordering};

use vault_query::index::{Builder, Note, NoteSource};

/// Counts the bytes live on the heap, and the most there were.
pub struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);

// SAFETY: forwards to the system allocator, counting.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = LIVE.fetch_add(layout.size().cast_signed(), Ordering::Relaxed) + layout.size().cast_signed();
        PEAK.fetch_max(now, Ordering::Relaxed);
        // SAFETY: the caller's contract, passed on.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size().cast_signed(), Ordering::Relaxed);
        // SAFETY: the caller's contract, passed on.
        unsafe { System.dealloc(ptr, layout) }
    }
}

/// Bytes live now.
pub fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

/// The peak since the last reset, and a reset to now.
pub fn peak() -> isize {
    PEAK.load(Ordering::Relaxed)
}

pub fn reset_peak() {
    PEAK.store(live(), Ordering::Relaxed);
}

const STATUSES: &[&str] = &["todo", "doing", "done", "blocked"];

/// `n` notes of about a kilobyte: properties, tags, links, headings,
/// tasks, prose; and one base.
pub fn notes(n: usize) -> Vec<(String, String)> {
    let prose = "Some words about the thing, a [[Link]] or two and a #tag here. ".repeat(12);
    let mut out: Vec<(String, String)> = (0..n)
        .map(|i| {
            let path = format!("Area {}/Note {i}.md", i % 20);
            let text = format!(
                "---\nstatus: {}\npriority: {}\ndue: 2026-{:02}-{:02}\ntags: [note, t{}]\nproject: \"[[Project {}]]\"\n---\n# Note {i}\n\n{prose}\n\n## Tasks\n- [ ] a task for {i}\n- [x] a done one\n",
                STATUSES[i % STATUSES.len()],
                i % 5,
                i % 12 + 1,
                i % 28 + 1,
                i % 7,
                i % 50,
            );
            (path, text)
        })
        .collect();
    out.push(("Open.base".to_owned(), BASE.to_owned()));
    out
}

pub const BASE: &str = "filters:\n  and:\n    - status != \"done\"\nformulas:\n  urgent: priority >= 3\nviews:\n  - type: table\n    name: Open\n    order: [file.name, status, priority, due, formula.urgent]\n    sort:\n      - property: due\n        direction: ASC\n    groupBy:\n      property: status\n      direction: ASC\n";

/// Notes held as markdown.
pub struct Markdown(pub Vec<(String, String)>);

impl NoteSource for Markdown {
    fn revision(&self) -> u64 {
        0
    }
    fn visit(&self, from: usize, n: usize, each: &mut dyn FnMut(Note<'_>)) -> (u64, usize) {
        for (path, text) in self.0.iter().skip(from).take(n) {
            each(Note {
                path,
                stamp: 1,
                text: vault_query::Text::Here(text),
                parsed: None,
                mtime: 1_763_000_000_000,
                is_base: std::path::Path::new(path).extension().is_some_and(|e| e.eq_ignore_ascii_case("base")),
            });
        }
        (0, self.0.len())
    }
}

/// The snapshot a node would send for `n` notes.
pub fn snapshot(n: usize) -> Vec<u8> {
    let mut b = Builder::new();
    let _ = b.sync(&Markdown(notes(n)));
    b.index().to_bytes()
}
