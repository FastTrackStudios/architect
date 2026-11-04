//! A WASI component that misbehaves on purpose (`tests/wasi.rs`): the
//! host's limits must hold against it, and the host run on. Its workers
//! misbehave too: one never ends and never looks whether it should, one
//! eats memory, one floods its owner, one never reads.

// A WASI component, built for wasm32-wasip2 only. On a native workspace build
// (`cargo test --workspace`) it is empty: its WIT exports have names like
// `architect:extension/guest@0.1.0#actions` that a native linker (mold) rejects.
#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use architect_extension_wasi::{Extension, Worker, host, worker};
use serde_json::{Value, json};

thread_local! {
    static HOARD: RefCell<Vec<Vec<u8>>> = const { RefCell::new(Vec::new()) };
    static WORKERS: RefCell<BTreeMap<u64, Worker>> = const { RefCell::new(BTreeMap::new()) };
}

#[allow(clippy::cast_precision_loss, clippy::as_conversions)] // A progress count's.
const fn lossy(n: u64) -> f64 {
    n as f64
}

const fn spin() -> ! {
    let mut n: u64 = 0;
    loop {
        n = std::hint::black_box(n.wrapping_add(1));
    }
}

fn id(args: &Value) -> u64 {
    args.get("id").and_then(Value::as_u64).unwrap_or(0)
}

fn build() -> Extension {
    Extension::new()
        // A call that never ends.
        .action("TEST_HAZARDS_SPIN", |_| spin())
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
        // Starts worker `worker` with `args`: its id.
        .action("TEST_HAZARDS_WORK", |args: Value| {
            let name = args.get("worker").and_then(Value::as_str).unwrap_or("");
            let w = worker::spawn(name, args.get("args").unwrap_or(&Value::Null))?;
            let id = w.id();
            WORKERS.with(|all| all.borrow_mut().insert(id, w));
            Ok(json!(id))
        })
        // Worker `id`'s state as its owner heard it.
        .action("TEST_HAZARDS_WORKER", |args: Value| {
            Ok(WORKERS.with(|all| {
                all.borrow()
                    .get(&id(&args))
                    .map_or_else(|| json!({ "status": "unknown" }), |w| w.state().to_json())
            }))
        })
        // Posts worker `id` `n` messages: how many went before one did not.
        .action("TEST_HAZARDS_FLOOD", |args: Value| {
            let n = args.get("n").and_then(Value::as_u64).unwrap_or(0);
            WORKERS.with(|all| {
                let all = all.borrow();
                let w = all.get(&id(&args)).ok_or("no such worker")?;
                for i in 0..n {
                    if let Err(why) = w.post(&json!(i)) {
                        return Ok(json!({ "accepted": i, "error": why }));
                    }
                }
                Ok(json!({ "accepted": n }))
            })
        })
        // Keeps its instance busy for `ms`.
        .action("TEST_HAZARDS_STALL", |args: Value| {
            let ms = args.get("ms").and_then(Value::as_u64).unwrap_or(0);
            let until = Instant::now()
                .checked_add(Duration::from_millis(ms))
                .unwrap_or_else(Instant::now);
            while Instant::now() < until {
                std::hint::black_box(0);
            }
            Ok(Value::Null)
        })
        // The app's interface (#306): a notification, the clipboard there
        // and back, a picker, a prompt, a menu, a badge; what came back.
        .action("TEST_HAZARDS_ASK", |_| {
            let e = |e: host::HostError| e.to_string();
            host::notify("hello from WASI", "success").map_err(e)?;
            host::clipboard_write("copied by WASI").map_err(e)?;
            let clip = host::clipboard_read().map_err(e)?;
            let path = host::pick(&json!({ "kind": "open" })).map_err(e)?;
            let name = host::prompt(&json!({ "kind": "text", "title": "Name?" })).map_err(e)?;
            let chosen =
                host::menu(&json!({ "x": 4, "y": 8, "items": [{ "id": "go", "label": "Go" }] }))
                    .map_err(e)?;
            host::decorate("file", "notes/a.md", Some(&json!({ "badge": "W" }))).map_err(e)?;
            Ok(json!({ "clip": clip, "path": path, "name": name, "chosen": chosen }))
        })
        // Never ends, never looks.
        .worker("forever", |_, _| spin())
        .worker("hoard", |_, _| {
            let mut kept = Vec::new();
            loop {
                kept.push(vec![1_u8; 1 << 20]);
                std::hint::black_box(&kept);
            }
        })
        // Posts its owner `n` messages, as fast as it may.
        .worker("chatty", |work, args: Value| {
            let n = args.get("n").and_then(Value::as_u64).unwrap_or(0);
            for i in 0..n {
                work.post(&json!(i))?;
                work.progress(lossy(i.saturating_add(1)), lossy(n), "");
            }
            Ok(json!(n))
        })
        // Never reads what it is sent; ends when cancelled.
        .worker("deaf", |work, _| {
            while !work.cancelled() {
                std::thread::sleep(Duration::from_millis(5));
            }
            Err("cancelled".to_owned())
        })
        // Answers each message with itself, until told `"stop"`.
        .worker("echo", |work, _| {
            loop {
                match work.receive(Duration::from_secs(5)) {
                    Some(Value::String(s)) if s == "stop" => return Ok(json!("stopped")),
                    Some(m) => work.post(&json!({ "echo": m }))?,
                    None => return Err("nothing came".to_owned()),
                }
            }
        })
}

architect_extension_wasi::export!(build);
