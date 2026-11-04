//! The lab's WASI component extension (`extension.toml`): a counter view
//! the lab draws in a pane, an action that asks the app for its themes
//! through the host service, and a job — counting primes — in a worker
//! (its `[[workers]]`: another instance of this component on the host's
//! pool), its progress bar moving while the counter keeps answering
//! clicks. Built for `wasm32-wasip2`.

// A WASI component, built for wasm32-wasip2 only. On a native workspace build
// (`cargo test --workspace`) it is empty: its WIT exports have names like
// `architect:extension/guest@0.1.0#actions` that a native linker (mold) rejects.
#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};

use architect_extension_wasi::{Extension, Work, WorkStatus, Worker, host, refresh, worker};
use dioxus::prelude::*;
use serde_json::{Value, json};

thread_local! {
    static COUNT: Cell<i64> = const { Cell::new(0) };
    static ASKED: RefCell<String> = const { RefCell::new(String::new()) };
    /// The primes job, while it runs and after (dropping it cancels it).
    static PRIMES: RefCell<Option<Worker>> = const { RefCell::new(None) };
}

/// How far the primes job counts when not told.
const PRIMES_BELOW: u64 = 2_000_000;

/// Counts the primes below `n`, saying how far it is a hundred times.
fn count_primes(work: &Work, n: u64) -> Result<u64, String> {
    let step = n.checked_div(100).unwrap_or(0).max(1);
    let mut found = 0_u64;
    for i in 2..n {
        if i.checked_rem(step) == Some(0) {
            work.check()?;
            work.progress(lossy(i), lossy(n), &format!("{found} primes below {i}"));
        }
        if is_prime(i) {
            found = found.saturating_add(1);
        }
    }
    Ok(found)
}

/// Whether `i` (2 or more) is prime: trial division, slow on purpose.
fn is_prime(i: u64) -> bool {
    let mut d: u64 = 2;
    while d.saturating_mul(d) <= i {
        if i.checked_rem(d) == Some(0) {
            return false;
        }
        d = d.saturating_add(1);
    }
    true
}

/// `n` for a progress bar.
#[allow(clippy::cast_precision_loss, clippy::as_conversions)] // A bar's precision.
const fn lossy(n: u64) -> f64 {
    n as f64
}

/// Starts the primes job (cancelling one running): its worker's handle.
fn start_primes(n: u64) -> Result<u64, String> {
    let w = worker::spawn("primes", &json!({ "n": n }))?;
    let id = w.id();
    // The old one, if any, is dropped (cancelled) here.
    PRIMES.with(|p| *p.borrow_mut() = Some(w));
    refresh();
    Ok(id)
}

/// The primes job's state, as JSON (`idle` before any).
fn primes_state() -> Value {
    PRIMES.with(|p| {
        p.borrow()
            .as_ref()
            .map_or_else(|| json!({ "status": "idle" }), |w| w.state().to_json())
    })
}

/// Adds `by` to the count; every open view shows it.
fn add(by: i64) -> i64 {
    let now = COUNT.with(|c| {
        c.set(c.get().saturating_add(by));
        c.get()
    });
    refresh();
    now
}

/// Asks the app for its themes (`THEME_LIST`): how many, or why not.
fn ask() -> Result<Value, String> {
    let said = host::run("THEME_LIST", &Value::Null);
    let shown = match &said {
        Ok(themes) => {
            let n = themes
                .as_array()
                .or_else(|| themes.get("themes").and_then(Value::as_array))
                .map_or(0, Vec::len);
            format!("The app has {n} themes")
        }
        Err(e) => format!("The app said no: {e}"),
    };
    host::log(host::Level::Info, &shown);
    ASKED.with(|a| *a.borrow_mut() = shown);
    refresh();
    said.map_err(|e| e.to_string())
}

#[component]
fn Counter() -> Element {
    let count = COUNT.with(Cell::get);
    let asked = ASKED.with(|a| a.borrow().clone());
    rsx! {
        div {
            style: "display: flex; flex-direction: column; align-items: center; justify-content: center; \
                    gap: 14px; height: 100%; font-family: -apple-system, system-ui, sans-serif; \
                    color: var(--foreground, #e4e4e7);",
            span { style: "font-size: 12px; letter-spacing: 0.08em; text-transform: uppercase; opacity: 0.7;",
                "A WASI component"
            }
            span { class: "wasi-count", style: "font-size: 56px; font-weight: 600;", "{count}" }
            div { style: "display: flex; gap: 8px;",
                button {
                    class: "wasi-add",
                    style: "padding: 6px 14px; border-radius: 6px; border: 1px solid var(--border, #3f3f46); \
                            background: var(--primary, #4f46e5); color: var(--primary-foreground, white); font-size: 13px;",
                    onclick: move |_| {
                        add(1);
                    },
                    "Add one"
                }
                button {
                    class: "wasi-ask",
                    style: "padding: 6px 14px; border-radius: 6px; border: 1px solid var(--border, #3f3f46); \
                            background: transparent; color: inherit; font-size: 13px;",
                    onclick: move |_| {
                        let _ = ask();
                    },
                    "Ask the app"
                }
            }
            if !asked.is_empty() {
                span { class: "wasi-asked", style: "font-size: 13px; opacity: 0.8;", "{asked}" }
            }
            Primes {}
        }
    }
}

/// The primes job: its button, its progress bar, what it found.
#[component]
fn Primes() -> Element {
    let state = PRIMES.with(|p| p.borrow().as_ref().map(Worker::state));
    let running = state.as_ref().is_some_and(|s| s.status == WorkStatus::Running);
    let percent = state.as_ref().map_or(0.0, |s| s.fraction() * 100.0);
    let said = state.as_ref().map_or_else(String::new, |s| match (&s.status, &s.result) {
        (WorkStatus::Done, Some(Ok(r))) => format!(
            "{} primes below {}",
            r.get("found").unwrap_or(&Value::Null),
            r.get("below").unwrap_or(&Value::Null)
        ),
        (WorkStatus::Failed | WorkStatus::Cancelled, Some(Err(why))) => format!("Stopped: {why}"),
        _ => format!("Counting primes: {percent:.0}%"),
    });
    rsx! {
        div { style: "display: flex; flex-direction: column; align-items: center; gap: 8px; width: 260px;",
            div { style: "display: flex; gap: 8px;",
                button {
                    class: "wasi-primes",
                    style: "padding: 6px 14px; border-radius: 6px; border: 1px solid var(--border, #3f3f46); \
                            background: transparent; color: inherit; font-size: 13px;",
                    onclick: move |_| {
                        let _ = start_primes(PRIMES_BELOW);
                    },
                    if running { "Count again" } else { "Count primes" }
                }
                if running {
                    button {
                        class: "wasi-primes-cancel",
                        style: "padding: 6px 14px; border-radius: 6px; border: 1px solid var(--border, #3f3f46); \
                                background: transparent; color: inherit; font-size: 13px;",
                        onclick: move |_| {
                            PRIMES.with(|p| {
                                if let Some(w) = p.borrow().as_ref() {
                                    w.cancel();
                                }
                            });
                        },
                        "Cancel"
                    }
                }
            }
            if state.is_some() {
                div {
                    class: "wasi-progress",
                    style: "width: 100%; height: 8px; border-radius: 4px; background: var(--muted, #27272a); overflow: hidden;",
                    div { style: "width: {percent}%; height: 100%; background: var(--primary, #4f46e5);" }
                }
                span { class: "wasi-primes-said", style: "font-size: 13px; opacity: 0.8;", "{said}" }
            }
        }
    }
}

fn build() -> Extension {
    Extension::new()
        .view("counter", Counter)
        .action("LAB_WASI_COUNTER_ADD", |args: Value| {
            let by = args.get("by").and_then(Value::as_i64).unwrap_or(1);
            Ok(json!(add(by)))
        })
        .action("LAB_WASI_COUNTER_ASK", |_| ask())
        .action("LAB_WASI_COUNTER_PRIMES", |args: Value| {
            let n = args.get("n").and_then(Value::as_u64).unwrap_or(PRIMES_BELOW);
            start_primes(n).map(|id| json!({ "worker": id }))
        })
        .action("LAB_WASI_COUNTER_JOB", |_| Ok(primes_state()))
        // The worker: another instance of this component, on the host's
        // pool; it shares nothing with the view's but messages.
        .worker("count-primes", |work, args: Value| {
            let n = args.get("n").and_then(Value::as_u64).unwrap_or(PRIMES_BELOW);
            count_primes(work, n).map(|found| json!({ "found": found, "below": n }))
        })
        .state(
            |saved| {
                if let Ok(bytes) = <[u8; 8]>::try_from(saved) {
                    COUNT.with(|c| c.set(i64::from_le_bytes(bytes)));
                }
            },
            || COUNT.with(|c| c.get().to_le_bytes().to_vec()),
        )
}

architect_extension_wasi::export!(build);
