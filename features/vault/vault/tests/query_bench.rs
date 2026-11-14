//! What the vault index and the query service cost on a large vault
//! (ignored: run it with `cargo test -p vault --features actions --release
//! --test query_bench -- --ignored --nocapture`). A browser's copy of the
//! index, in V8: `features/vault/vault-query/examples/index_memory.mjs`.
//!
//! Two generated vaults of `QUERY_BENCH_NOTES` notes (10 000 by default):
//! an even one (about a kilobyte a note: properties, tags, links,
//! headings, tasks, prose) and a skewed one (one note in a hundred has 120
//! properties and a long body; every note links to one note; most notes
//! share one status). Queries go through the service's client
//! (`VaultQueryClient`, served in process), so each answer is serialized
//! and read as it is for an extension.
//!
//! - The index: built from the open vault, a snapshot's bytes, its bytes
//!   in memory (counted by this test's allocator).
//! - A delta for a typical edit (one property set), and a sync after it.
//! - A query: every matching row, `limit: 50`, `if_changed` unchanged, and
//!   after an edit.
//! - What an editor's save waits for the vault's lock, with no syncs and
//!   with syncs and queries running on two other threads.

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
use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::json;
use vault::actions::OpenVault;
use vault::query::{Built, QueryEngine};
use vault_proto::query::{QueryRequest, VaultQueryClient};

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

fn generate(root: &std::path::Path, n: usize, skewed: bool) {
    let prose = "Some words about the thing, a [[Link]] or two and a #tag here. ".repeat(12);
    for i in 0..n {
        let folder = root.join(format!("Area {}", i % 20));
        std::fs::create_dir_all(&folder).unwrap();
        let heavy = skewed && i % 100 == 0;
        let status = if skewed && i % 10 < 7 { "todo" } else { STATUSES[i % STATUSES.len()] };
        let mut extra = String::new();
        if heavy {
            for k in 0..120 {
                let _ = writeln!(extra, "field{k}: value {k} of note {i}");
            }
        }
        let body = if heavy { prose.repeat(20) } else { prose.clone() };
        let hot = if skewed { "[[Hot]] " } else { "" };
        let text = format!(
            "---\nstatus: {status}\npriority: {}\ndue: 2026-{:02}-{:02}\ntags: [note, t{}]\nproject: \"[[Project {}]]\"\n{extra}---\n# Note {i}\n\n{hot}{body}\n\n## Tasks\n- [ ] a task for {i}\n- [x] a done one\n",
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
    println!("| {what} | {} | {:.2} ms | {:.2} ms |", v.len(), at(0.5), at(0.99));
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
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    for skewed in [false, true] {
        println!("\n## {} vault, {n} notes\n", if skewed { "A skewed" } else { "An even" });
        let dir = tempfile::tempdir().unwrap();
        generate(dir.path(), n, skewed);
        let (open_t, open) = time(|| OpenVault::open(dir.path()).unwrap());
        let text_bytes: usize = open.with(|v| v.pages.iter().map(|p| p.raw.len()).sum());
        println!("{:.1} MiB of markdown, opened in {:.0} ms\n", mib(text_bytes as isize), ms(open_t));

        let before_index = live();
        let built = Arc::new(Built::new(open.clone()));
        let (build_t, _) = time(|| built.sync());
        let index = built.index();
        let index_mem = live() - before_index;
        let wire = index.to_bytes();
        drop(index);
        let engine = QueryEngine::new(built.clone());
        let scope = architect::resource::Scope::new();
        let client: VaultQueryClient = rt
            .block_on(architect::serve_local(engine, &scope).establish())
            .unwrap();
        let ask = |q: &QueryRequest| {
            let text = rt.block_on(client.query(q.clone())).unwrap();
            // As the host reads it for an extension.
            let v: serde_json::Value = serde_json::from_str(&text).unwrap();
            (text.len(), v)
        };
        let q = QueryRequest {
            path: Some("Open.base".to_owned()),
            ..QueryRequest::default()
        };

        let mut deltas = Vec::new();
        let mut syncs = Vec::new();
        let mut after = Vec::new();
        let small = QueryRequest { limit: Some(50), ..q.clone() };
        for i in 0..20 {
            let path = format!("Area {}/Note {}.md", i % 20, i * 20 + i % 20);
            open.set_property(&path, "status", json!(STATUSES[(i + 1) % 4])).unwrap();
            let (t, d) = time(|| built.sync());
            syncs.push(t);
            deltas.push(d.to_bytes().len());
            after.push(time(|| ask(&small)).0);
        }
        deltas.sort_unstable();
        println!("| the index | |\n|---|---:|");
        println!("| built from the open vault | {:.0} ms |", ms(build_t));
        println!("| a snapshot on the wire (JSON) | {:.2} MiB ({} B a note) |", mib(wire.len() as isize), wire.len() / n);
        println!("| in memory (entries, stamps) | {:.2} MiB ({} B a note) |", mib(index_mem), index_mem / n as isize);
        println!("| a delta for one property set (wire) | {} B |\n", deltas[deltas.len() / 2]);

        println!("| query, through the client | n | p50 | p99 |\n|---|---:|---:|---:|");
        let (bytes, first) = ask(&q);
        let shown = first["total"].as_u64().unwrap();
        let mut all: Vec<_> = (0..20).map(|_| time(|| ask(&q)).0).collect();
        row(&format!("every matching row ({shown} rows, {:.1} MiB answered)", mib(bytes as isize)), &mut all);
        let mut limited: Vec<_> = (0..50).map(|_| time(|| ask(&small)).0).collect();
        row("`limit: 50` (an extension's page; 200 by default)", &mut limited);
        let hash = ask(&small).1["hash"].as_str().unwrap().to_owned();
        let same = QueryRequest { if_changed: Some(hash), ..small.clone() };
        let mut unchanged: Vec<_> = (0..50).map(|_| time(|| ask(&same)).0).collect();
        row("`if_changed`, nothing moved", &mut unchanged);
        row("index sync after one edit", &mut syncs);
        row("`limit: 50` after one edit (the whole view again)", &mut after);

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
                let stop = Arc::clone(&stop);
                let engine = QueryEngine::new(built.clone());
                let small = small.clone();
                std::thread::spawn(move || {
                    let mut k = 0_u32;
                    while !stop.load(Ordering::Relaxed) {
                        let _ = engine.run_text(&small).unwrap();
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
        drop(client);
        rt.block_on(scope.close());
    }
}
