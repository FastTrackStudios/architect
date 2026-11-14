//! A browser's copy of the vault index in memory, natively (ignored: run
//! it with `cargo test -p vault-query --release --test memory -- --ignored
//! --nocapture`). The same in a browser's engine: `examples/index_memory.mjs`.

#![allow(clippy::unwrap_used, clippy::print_stdout, clippy::cast_precision_loss, clippy::cast_possible_wrap, clippy::as_conversions, clippy::arithmetic_side_effects)]

mod support;

use std::sync::Arc;
use std::time::Instant;

use support::measure::{self, Counting};
use vault_proto::query::QueryRequest;
use vault_query::{QueryEngine, Replica};

#[global_allocator]
static ALLOC: Counting = Counting;

#[test]
#[ignore = "a measurement: run it with --ignored --nocapture, in release"]
fn a_browsers_copy_of_the_index() {
    let n = 10_000;
    let bytes = measure::snapshot(n);
    let before = measure::live();
    let replica = Arc::new(Replica::from_bytes(&bytes).unwrap());
    let held = measure::live() - before;
    let engine = QueryEngine::new(replica);
    let q = QueryRequest {
        path: Some("Open.base".to_owned()),
        limit: Some(50),
        ..QueryRequest::default()
    };
    measure::reset_peak();
    let at_query = measure::live();
    let t = Instant::now();
    let answer = engine.run(&q).unwrap();
    let first = t.elapsed();
    let peak = measure::peak() - at_query;
    let kept = measure::live() - at_query;
    let t = Instant::now();
    let _ = engine.run(&q).unwrap();
    let second = t.elapsed();
    println!("native, {n} notes: snapshot {} B ({} B a note); replica holds {} B ({} B a note); a query (limit 50, total {}) peaks at {} B, keeps {} B; first {:.1} ms, again {:.1} ms",
        bytes.len(), bytes.len() / n, held, held / n as isize, answer["total"], peak, kept,
        first.as_secs_f64() * 1e3, second.as_secs_f64() * 1e3);
}
