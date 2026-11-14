//! What the vault index and the query service cost on a large vault
//! (ignored: run it with `cargo test -p vault --features actions --release
//! --test query_bench -- --ignored --nocapture`).
//!
//! A generated vault of `QUERY_BENCH_NOTES` notes (10 000 by default),
//! each about a kilobyte: properties, tags, links, headings, tasks, prose.
//!
//! - The index: its bytes on the wire (a snapshot), its bytes in memory
//!   (counted by this test's allocator), and how long the first build takes.
//! - A delta: its bytes on the wire for a typical edit (one property set),
//!   and how long a sync takes.
//! - A query: on an index that has not moved, one that only asks whether
//!   its answer changed, and the first one after an edit.
//! - What an editor's save waits for the vault's lock, with no index
//!   syncs and with syncs and queries running on two other threads.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    clippy::indexing_slicing,
    clippy::cast_possible_wrap,
    clippy::arithmetic_side_effects,
    // The query threads must be running before the saves are timed.
    clippy::needless_collect
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;
use vault::actions::OpenVault;
use vault::query::Built;
use vault::query::QueryEngine;
use vault_proto::query::QueryRequest;

/// Counts the bytes live on the heap.
struct Counting;

static LIVE: AtomicIsize = AtomicIsize::new(0);

// SAFETY: forwards to the system allocator, counting.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: the caller's contract, passed on.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        // SAFETY: the caller's contract, passed on.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

const STATUSES: &[&str] = &["todo", "doing", "done", "blocked"];

fn generate(root: &std::path::Path, n: usize) {
    let prose = "Some words about the thing, a [[Link]] or two and a #tag here. ".repeat(12);
    for i in 0..n {
        let folder = root.join(format!("Area {}", i % 20));
        std::fs::create_dir_all(&folder).unwrap();
        let text = format!(
            "---\nstatus: {}\npriority: {}\ndue: 2026-{:02}-{:02}\ntags: [note, t{}]\nproject: \"[[Project {}]]\"\n---\n# Note {i}\n\n{prose}\n\n## Tasks\n- [ ] a task for {i}\n- [x] a done one\n",
            STATUSES[i % STATUSES.len()],
            i % 5,
            i % 12 + 1,
            i % 28 + 1,
            i % 7,
            i % 50,
        );
        std::fs::write(folder.join(format!("Note {i}.md")), text).unwrap();
    }
    std::fs::write(root.join("Open.base"), BASE).unwrap();
}

const BASE: &str = "filters:\n  and:\n    - status != \"done\"\nformulas:\n  urgent: priority >= 3\nviews:\n  - type: table\n    name: Open\n    order: [file.name, status, priority, due, formula.urgent]\n    sort:\n      - property: due\n        direction: ASC\n    groupBy:\n      property: status\n      direction: ASC\n";

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn row(what: &str, v: &mut [Duration]) {
    v.sort();
    let at = |q: f64| ms(v[((v.len() - 1) as f64 * q) as usize]);
    println!(
        "| {what} | {} | {:.2} ms | {:.2} ms | {:.2} ms |",
        v.len(),
        at(0.5),
        at(0.99),
        ms(v[v.len() - 1])
    );
}

fn time<R>(f: impl FnOnce() -> R) -> (Duration, R) {
    let t = Instant::now();
    let r = f();
    (t.elapsed(), r)
}

fn mib(b: isize) -> f64 {
    b as f64 / 1024.0 / 1024.0
}

#[test]
#[ignore = "a benchmark: run it with --ignored --nocapture, in release"]
fn the_index_and_queries_on_a_large_vault() {
    let n: usize = std::env::var("QUERY_BENCH_NOTES")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10_000);
    let dir = tempfile::tempdir().unwrap();
    generate(dir.path(), n);
    let before_open = live();
    let (open_t, open) = time(|| OpenVault::open(dir.path()).unwrap());
    let vault_bytes = live() - before_open;
    let text_bytes: usize = open.with(|v| v.pages.iter().map(|p| p.raw.len()).sum());
    println!(
        "{n} notes ({:.1} MiB of markdown), opened in {:.0} ms; the open vault holds {:.1} MiB\n",
        mib(text_bytes as isize),
        ms(open_t),
        mib(vault_bytes)
    );

    // The index.
    let before_index = live();
    let built = Arc::new(Built::new(open.clone()));
    let (build_t, _) = time(|| built.sync());
    let index = built.index();
    let index_mem = live() - before_index;
    let wire = index.to_bytes();
    let engine = QueryEngine::new(built.clone());
    let q = QueryRequest {
        path: Some("Open.base".to_owned()),
        ..QueryRequest::default()
    };
    let before_rows = live();
    let (cold, first) = time(|| engine.run(&q).unwrap());
    let rows_mem = live() - before_rows;
    println!("| the index | |\n|---|---:|");
    println!("| built from the open vault | {:.0} ms |", ms(build_t));
    println!("| on the wire (a snapshot, JSON) | {:.2} MiB ({} B a note) |", mib(wire.len() as isize), wire.len() / n);
    println!("| in memory (the entries, the builder's stamps) | {:.2} MiB ({} B a note) |", mib(index_mem), index_mem / n as isize);
    println!("| the engine's rows, kept between queries | {:.2} MiB ({} B a note) |", mib(rows_mem), rows_mem / n as isize);

    // A typical edit: one property set on one note.
    let mut deltas = Vec::new();
    let mut syncs = Vec::new();
    let mut after = Vec::new();
    for i in 0..20 {
        let path = format!("Area {}/Note {}.md", i % 20, i * 20 + i % 20);
        open.set_property(&path, "status", json!(STATUSES[(i + 1) % 4])).unwrap();
        let (t, d) = time(|| built.sync());
        syncs.push(t);
        deltas.push(d.to_bytes().len());
        after.push(time(|| engine.run(&q).unwrap()).0);
    }
    deltas.sort_unstable();
    println!("| a delta for one property set (wire) | {} B (p50), {} B (max) |\n", deltas[deltas.len() / 2], deltas[deltas.len() - 1]);

    println!("| what | n | p50 | p99 | max |\n|---|---:|---:|---:|---:|");
    row("first query (rows made for every note)", &mut [cold]);
    let shown = first["total"].as_u64().unwrap();
    let mut warm: Vec<_> = (0..50).map(|_| time(|| engine.run(&q).unwrap()).0).collect();
    row(&format!("query, index not moved ({shown} rows)"), &mut warm);
    let small = QueryRequest { limit: Some(50), ..q.clone() };
    let mut limited: Vec<_> = (0..50).map(|_| time(|| engine.run(&small).unwrap()).0).collect();
    row("query, `limit: 50`", &mut limited);
    let hash = engine.run(&q).unwrap()["hash"].as_str().unwrap().to_owned();
    let same = QueryRequest { if_changed: Some(hash), ..q.clone() };
    let mut unchanged: Vec<_> = (0..50).map(|_| time(|| engine.run(&same).unwrap()).0).collect();
    row("query, `if_changed` and unchanged", &mut unchanged);
    row("index sync after one edit", &mut syncs);
    row("query after one edit", &mut after);

    // What an editor's save waits for.
    let writes = |label: &str| {
        let mut v = Vec::new();
        for i in 0..200 {
            let path = format!("Area {}/Note {}.md", i % 20, (i * 20 + i % 20) % n);
            v.push(time(|| open.set_property(&path, "seen", json!(i)).unwrap()).0);
        }
        row(label, &mut v);
    };
    writes("a save, nothing else running");
    let stop = Arc::new(AtomicBool::new(false));
    let runners: Vec<_> = (0..2)
        .map(|_| {
            let (engine, q, stop) = (engine.clone(), q.clone(), Arc::clone(&stop));
            std::thread::spawn(move || {
                let mut k = 0_u32;
                while !stop.load(Ordering::Relaxed) {
                    let _ = engine.run(&q).unwrap();
                    k += 1;
                }
                k
            })
        })
        .collect();
    writes("a save, with index syncs and queries on two threads");
    stop.store(true, Ordering::Relaxed);
    let ran: u32 = runners.into_iter().map(|t| t.join().unwrap()).sum();
    println!("\n({ran} queries, each syncing the index first, ran beside those saves)");
}
