//! The lab's WASI component extension (`extension.toml`): a counter view
//! the lab draws in a pane, and an action that asks the app for its
//! themes through the host service. Built for `wasm32-wasip2`.

use std::cell::{Cell, RefCell};

use architect_extension_wasi::{Extension, host, refresh};
use dioxus::prelude::*;
use serde_json::{Value, json};

thread_local! {
    static COUNT: Cell<i64> = const { Cell::new(0) };
    static ASKED: RefCell<String> = const { RefCell::new(String::new()) };
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
