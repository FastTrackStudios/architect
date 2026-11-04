//! A WASI component that misbehaves on purpose (`tests/wasi.rs`): the
//! host's limits must hold against it, and the host run on.

// A WASI component, built for wasm32-wasip2 only. On a native workspace build
// (`cargo test --workspace`) it is empty: its WIT exports have names like
// `architect:extension/guest@0.1.0#actions` that a native linker (mold) rejects.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;

use architect_extension_wasi::Extension;
use serde_json::{Value, json};

thread_local! {
    static HOARD: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
}

fn build() -> Extension {
    Extension::new()
        // A call that never ends.
        .action("TEST_HAZARDS_SPIN", |_| {
            let mut n: u64 = 0;
            loop {
                n = std::hint::black_box(n.wrapping_add(1));
            }
        })
        // Memory, a megabyte at a time, kept.
        .action("TEST_HAZARDS_HOARD", |_| {
            loop {
                HOARD.with(|h| h.borrow_mut().push(vec![1_u8; 1 << 20]));
            }
        })
        .action("TEST_HAZARDS_ECHO", |args: Value| {
            println!("echo {args}");
            Ok(json!({ "echo": args }))
        })
}

architect_extension_wasi::export!(build);
