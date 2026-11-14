//! A browser's copy of the vault index, in a browser's engine: built to
//! wasm (`wasm32-unknown-unknown`, `wasm-bindgen --target nodejs`) and run
//! by `examples/index_memory.mjs` under Node (V8). What it measures is
//! `tests/memory.rs`'s, natively.

#![allow(
    clippy::unwrap_used,
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::as_conversions,
    clippy::missing_panics_doc,
    clippy::must_use_candidate,
    clippy::arithmetic_side_effects,
    clippy::indexing_slicing
)]

#[path = "../tests/support/measure.rs"]
mod measure;

use std::cell::RefCell;
use std::sync::Arc;

use vault_proto::query::QueryRequest;
use vault_query::{QueryEngine, Replica};
use wasm_bindgen::prelude::wasm_bindgen;

#[global_allocator]
static ALLOC: measure::Counting = measure::Counting;

thread_local! {
    static BYTES: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    static ENGINE: RefCell<Option<QueryEngine>> = const { RefCell::new(None) };
}

/// Builds `n` notes' snapshot (what the node would send): its length.
#[wasm_bindgen]
pub fn build(n: u32) -> u32 {
    let bytes = measure::snapshot(n as usize);
    let len = bytes.len() as u32;
    BYTES.with(|b| *b.borrow_mut() = bytes);
    len
}

/// Bytes live on the heap now.
#[wasm_bindgen]
pub fn live() -> f64 {
    measure::live() as f64
}

/// Loads the snapshot as a replica and an engine over it: the bytes they
/// hold.
#[wasm_bindgen]
pub fn load() -> f64 {
    let before = measure::live();
    let replica = BYTES.with(|b| Replica::from_bytes(&b.borrow()).unwrap());
    let engine = QueryEngine::new(Arc::new(replica));
    ENGINE.with(|e| *e.borrow_mut() = Some(engine));
    (measure::live() - before) as f64
}

/// Drops the snapshot's bytes (a browser keeps only the replica).
#[wasm_bindgen]
pub fn drop_bytes() {
    BYTES.with(|b| *b.borrow_mut() = Vec::new());
}

/// Runs the base's view with `limit`: `[total, peak bytes, kept bytes]`.
#[wasm_bindgen]
pub fn query(limit: u32) -> Vec<f64> {
    measure::reset_peak();
    let at = measure::live();
    let q = QueryRequest {
        path: Some("Open.base".to_owned()),
        limit: Some(limit),
        ..QueryRequest::default()
    };
    let total = ENGINE.with(|e| e.borrow().as_ref().unwrap().run(&q).unwrap()["total"].as_f64().unwrap_or(0.0));
    vec![total, (measure::peak() - at) as f64, (measure::live() - at) as f64]
}
