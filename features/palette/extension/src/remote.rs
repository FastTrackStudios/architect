//! The Command Palette's views as a process draws them.
//!
//! A development copy (`EXT_CLONE {"id":"command-palette"}`) has no
//! `Commands`, so it draws a palette of its own over the app's actions:
//! `COMMANDS_LIST` for the rows, ranked by the same engine as the
//! built-in, `COMMANDS_RUN` to run one, `COMMANDS_SHOW {overlay: none}`
//! to close. The shell's slot draws
//! it where the built-in's goes. Edit this file in the copy and the
//! palette the app shows changes as it rebuilds.

use architect_palette::{Recents, rank};
use dioxus::prelude::*;
use serde_json::json;

use crate::{handle, listed, run};

/// Closes the overlay.
fn close() {
    if let Some(h) = handle() {
        spawn(async move {
            let _ = run(&h, "COMMANDS_SHOW", json!({ "overlay": "none" })).await;
        });
    }
}

/// ⌘K, a development copy's: the app's commands, matched as typed.
#[component]
pub fn RemotePalette() -> Element {
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);
    let list = use_resource(|| async move {
        let h = handle()?;
        run(&h, "COMMANDS_LIST", json!({})).await.ok()
    });
    let items = list
        .read()
        .clone()
        .flatten()
        .map(|l| listed(&l))
        .unwrap_or_default();
    let rows = rank(&items, &query.read(), &Recents::default());
    let count = rows.len();
    let pick = selected().min(count.saturating_sub(1));
    let ids: Vec<(String, bool)> = rows
        .iter()
        .map(|r| (r.item.id.clone(), r.item.enabled))
        .collect();
    let run_at = move |i: usize| {
        let Some((id, true)) = ids.get(i).cloned() else {
            return;
        };
        if let Some(h) = handle() {
            spawn(async move {
                let _ = run(&h, "COMMANDS_SHOW", json!({ "overlay": "none" })).await;
                let _ = run(&h, "COMMANDS_RUN", json!({ "id": id })).await;
            });
        }
    };
    let run_key = run_at.clone();
    rsx! {
        div { style: "flex: 1; min-height: 0; display: flex; flex-direction: column; font-size: 13px; \
                      color: var(--foreground); background: var(--card);",
            div { style: "flex-shrink: 0; display: flex; align-items: center; gap: 8px; height: 44px; padding: 0 14px; \
                          border-bottom: 1px solid var(--border);",
                input {
                    r#type: "text",
                    value: "{query}",
                    "aria-label": "Type a command…",
                    placeholder: "Type a command… (development copy)",
                    style: "flex: 1; border: none; outline: none; background: transparent; color: var(--foreground); font-size: 15px;",
                    oninput: move |e| {
                        query.set(e.value());
                        selected.set(0);
                    },
                    onkeydown: move |e: KeyboardEvent| match e.key() {
                        Key::Named(NamedKey::ArrowDown) => selected.set(pick.saturating_add(1).min(count.saturating_sub(1))),
                        Key::Named(NamedKey::ArrowUp) => selected.set(pick.saturating_sub(1)),
                        Key::Named(NamedKey::Enter) => run_key(pick),
                        Key::Named(NamedKey::Escape) => close(),
                        _ => {}
                    },
                    onmounted: move |e| {
                        spawn(async move {
                            let _ = e.data().set_focus(true).await;
                        });
                    },
                }
                span { style: "font-size: 11px; color: var(--muted-foreground);", "dev copy" }
            }
            div { style: "flex: 1; min-height: 0; overflow-y: scroll; padding: 6px;",
                if rows.is_empty() {
                    div { style: "padding: 18px 12px; color: var(--muted-foreground);", "No command matches" }
                }
                for (i, r) in rows.into_iter().enumerate() {
                    div {
                        key: "{r.item.id}",
                        "data-command": "{r.item.id}",
                        style: if i == pick {
                            "height: 34px; display: flex; align-items: center; gap: 8px; padding: 0 10px; border-radius: 8px; background: var(--accent);"
                        } else {
                            "height: 34px; display: flex; align-items: center; gap: 8px; padding: 0 10px; border-radius: 8px;"
                        },
                        onclick: {
                            let run_at = run_at.clone();
                            move |_| run_at(i)
                        },
                        if !r.item.group.is_empty() {
                            span { style: "color: var(--muted-foreground);", "{r.item.group}:" }
                        }
                        span { "{r.item.label}" }
                    }
                }
            }
        }
    }
}

/// ⌘O, a development copy's: openers live in the app, so it says so and
/// closes on Esc.
#[component]
pub fn RemoteQuickOpen() -> Element {
    rsx! {
        div {
            tabindex: "0",
            style: "flex: 1; display: flex; align-items: center; justify-content: center; padding: 24px; \
                    font-size: 13px; color: var(--muted-foreground); background: var(--card);",
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    close();
                }
            },
            "Open anything is the app's: this development copy draws ⌘K. Esc closes."
        }
    }
}
