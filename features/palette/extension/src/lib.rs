//! The Command Palette (#381): ⌘K and ⌘O as a built-in extension.
//!
//! The shell (`architect-commands`) keeps only the palette slot
//! (`CommandLayer`), the commands and `COMMANDS_SHOW`; this extension
//! draws both overlays with its views (`[[palette_views]]`), binds their
//! keys (`[[keys]]`: ⌘K, ⇧⌘P, ⌘O and their Control forms) and ranks with
//! the palette engine (`architect-palette`). One value, [`CommandPalette`]:
//!
//! - **linked in** (an app's built-in), its views are [`views::PaletteView`]
//!   and [`views::QuickOpenView`], over the app's `Commands`, drawn with
//!   the commands' kit (`architect_commands::kit`);
//! - **as a process** (a development copy, `EXT_CLONE {"id":"command-palette"}`),
//!   its views are [`remote`]'s: a palette of its own over `COMMANDS_LIST`
//!   and `COMMANDS_RUN`, ranked by the same engine, in the built-in's place.
//!
//! Turn it off and ⌘K and ⌘O are gone (`COMMANDS_SHOW` says no palette
//! draws them). To replace it, clone it, or install an extension that
//! draws the `palette` and `quick-open` overlays and binds the keys.
//! Other extensions reach it as the `architect.palette` service:
//! `palette.show` and `palette.search` ([`SHOW_ACTION`], [`SEARCH_ACTION`]).

use std::sync::{Mutex, PoisonError};

use architect_extensions::hostcall::HostRequest;
use architect_extensions::linked::{Builder, Extension, Handle};
use architect_palette::{Item, Recents, rank};
use dioxus::prelude::*;
use serde_json::{Value, json};

pub mod remote;
pub mod views;

/// Its manifest.
pub const MANIFEST: &str = include_str!("../extension.toml");

/// Its id.
pub const ID: &str = "command-palette";

/// Opens ⌘K (its `[[keys]]`: ⌘K, ⌃K, ⇧⌘P, ⌃⇧P).
pub const OPEN_ACTION: &str = "COMMAND_PALETTE_OPEN";

/// Opens ⌘O (⌘O, ⌃O).
pub const OPEN_ANYTHING_ACTION: &str = "COMMAND_PALETTE_OPEN_ANYTHING";

/// `palette.show`: `COMMANDS_SHOW` for another extension.
pub const SHOW_ACTION: &str = "COMMAND_PALETTE_SHOW";

/// `palette.search`: the commands matching a query, as the palette ranks
/// them.
pub const SEARCH_ACTION: &str = "COMMAND_PALETTE_SEARCH";

/// How many rows `palette.search` answers unless asked.
const LIMIT: usize = 20;

/// The Command Palette.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommandPalette;

/// The host it was built for: its process views ask through it.
static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

/// The host this extension was built for.
pub(crate) fn handle() -> Option<Handle> {
    HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Runs the app's action `action` with `args`.
pub(crate) async fn run(host: &Handle, action: &str, args: Value) -> Result<Value, String> {
    host.host(HostRequest::Run {
        action: action.to_owned(),
        args,
        ticket: None,
    })
    .await
    .map_err(|e| format!("{action}: {e}"))
}

/// `COMMANDS_LIST`'s answer as the engine's items.
#[must_use]
pub fn listed(list: &Value) -> Vec<Item> {
    list.get("commands")
        .and_then(Value::as_array)
        .map(|all| {
            all.iter()
                .filter_map(|c| {
                    let text = |k: &str| c.get(k).and_then(Value::as_str).unwrap_or_default();
                    let id = text("id");
                    if id.is_empty() {
                        return None;
                    }
                    let item = Item::new(id, text("label")).group(text("group"));
                    let enabled = c.get("enabled").and_then(Value::as_bool).unwrap_or(true);
                    Some(if enabled {
                        item
                    } else {
                        let why = c
                            .get("disabled_reason")
                            .and_then(Value::as_str)
                            .unwrap_or("not now");
                        item.disabled(why)
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `palette.search`'s answer: the commands of `list` (`COMMANDS_LIST`'s)
/// matching `query`, best first, at most `limit`.
#[must_use]
pub fn search(list: &Value, query: &str, limit: usize) -> Value {
    let items = listed(list);
    let rows: Vec<Value> = rank(&items, query, &Recents::default())
        .into_iter()
        .take(limit)
        .map(|r| {
            json!({
                "id": r.item.id,
                "label": r.item.label,
                "group": r.item.group,
                "enabled": r.item.enabled,
                "positions": r.positions,
            })
        })
        .collect();
    json!({ "items": rows })
}

/// Its `palette` view: linked in, the app's palette; in a process, its own.
#[component]
fn Palette() -> Element {
    if architect_commands::try_use_commands().is_some() {
        rsx! { views::PaletteView {} }
    } else {
        rsx! { remote::RemotePalette {} }
    }
}

/// Its `quick-open` view.
#[component]
fn QuickOpen() -> Element {
    if architect_commands::try_use_commands().is_some() {
        rsx! { views::QuickOpenView {} }
    } else {
        rsx! { remote::RemoteQuickOpen {} }
    }
}

impl Extension for CommandPalette {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        let host = ext.handle();
        *HANDLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(host.clone());
        ext.view("palette", Palette);
        ext.view("quick-open", QuickOpen);
        for (id, overlay) in [
            (OPEN_ACTION, "palette"),
            (OPEN_ANYTHING_ACTION, "quick-open"),
        ] {
            let h = host.clone();
            ext.action(id, move |_| {
                let h = h.clone();
                async move { run(&h, "COMMANDS_SHOW", json!({ "overlay": overlay })).await }
            });
        }
        let h = host.clone();
        ext.action(SHOW_ACTION, move |args: Value| {
            let h = h.clone();
            async move {
                let overlay = args
                    .get("overlay")
                    .and_then(Value::as_str)
                    .unwrap_or("palette")
                    .to_owned();
                let mut ask = json!({ "overlay": overlay });
                if let Some(q) = args.get("query").and_then(Value::as_str) {
                    ask["query"] = json!(q);
                }
                run(&h, "COMMANDS_SHOW", ask).await
            }
        });
        let h = host;
        ext.action(SEARCH_ACTION, move |args: Value| {
            let h = h.clone();
            async move {
                let query = args
                    .get("query")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .and_then(|n| usize::try_from(n).ok())
                    .unwrap_or(LIMIT);
                let list = run(&h, "COMMANDS_LIST", json!({})).await?;
                Ok(search(&list, &query, limit))
            }
        });
    }
}
