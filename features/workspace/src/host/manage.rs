//! The sheet that manages the saved layouts (Obsidian's "Manage workspace
//! layouts"): every set with its slot, whether it is the active one and
//! has changes; per set Load, Save over, Rename, its slot, Delete (not
//! for a built-in one), Reset (the active one, changed); and a field to save the live layout under
//! a name (over the set of that name, if there is one).
//!
//! Opened by `WORKSPACES_MANAGE` (the palette, the app's View menu, a
//! ribbon button). Every button runs its `WORKSPACES_*` action, so the
//! sheet, a key and an agent do the same.

use dioxus::prelude::*;

use super::{Host, use_host};
use crate::control::{self, DeleteArgs, LoadArgs, RenameArgs, SaveArgs, SlotArgs};

/// Over everything but the palette.
const Z_SHEET: u8 = 60;

/// A row as the sheet shows it.
#[derive(Clone, PartialEq)]
struct Row {
    id: String,
    name: String,
    slot: Option<u8>,
    builtin: bool,
    active: bool,
}

/// The next slot after `now`: 1–9, then none.
const fn next_slot(now: Option<u8>) -> Option<u8> {
    match now {
        None => Some(1),
        Some(n) if n >= 9 => None,
        Some(n) => Some(n.saturating_add(1)),
    }
}

/// Closes the sheet.
fn close(host: Host) {
    let mut managing = host.managing;
    managing.set(false);
}

#[component]
pub(super) fn ManageSheet() -> Element {
    let host = use_host();
    if !(host.managing)() {
        return rsx! {};
    }
    let t = *host.theme.read();
    let noun = host.sets_noun.read().clone();
    let lower = noun.to_lowercase();
    let (rows, dirty) = {
        let sets = host.screensets.read();
        let rows: Vec<Row> = sets
            .sets
            .iter()
            .map(|s| Row {
                id: s.id.clone(),
                name: s.name.clone(),
                slot: s.slot,
                builtin: s.builtin,
                active: sets.active.as_deref() == Some(s.id.as_str()),
            })
            .collect();
        (rows, sets.is_dirty(*host.form.read()))
    };
    let mut name = use_signal(String::new);
    let save_new = move || {
        let typed = name.peek().trim().to_owned();
        if !typed.is_empty() {
            host.write(
                control::save,
                SaveArgs {
                    if_revision: None,
                    name: Some(typed),
                },
            );
            name.set(String::new());
        }
    };
    let mut on_enter = save_new;
    rsx! {
        div {
            "data-workspaces-sheet": "",
            style: "position:absolute; top:0; left:0; right:0; bottom:0; z-index:{Z_SHEET}; background:{t.scrim}; \
                    display:flex; align-items:center; justify-content:center;",
            onclick: move |_| close(host),
            onkeydown: move |e| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    close(host);
                }
            },
            div {
                role: "dialog",
                "aria-label": "Manage {lower} layouts",
                style: "width:520px; max-width:92%; max-height:85%; overflow-y:scroll; box-sizing:border-box; \
                        display:flex; flex-direction:column; gap:12px; padding:18px; border-radius:12px; \
                        background:{t.pill}; border:1px solid {t.rule}; box-shadow:{t.shadow_float}; color:{t.ink};",
                onclick: move |e| e.stop_propagation(),
                div { style: "display:flex; align-items:center;",
                    div { style: "flex:1; font-size:15px; font-weight:600;", "Manage {lower} layouts" }
                    button {
                        "data-workspaces-close": "",
                        style: "border:none; background:transparent; color:{t.ink_2}; font-size:15px; cursor:pointer;",
                        onclick: move |_| close(host),
                        "✕"
                    }
                }
                if rows.is_empty() {
                    div { style: "color:{t.ink_3}; font-size:13px;", "No saved {lower} layouts yet." }
                }
                for row in rows {
                    SetRow { key: "{row.id}", row: row.clone(), dirty: dirty && row.active }
                }
                div { style: "display:flex; gap:8px; align-items:center; padding-top:6px; border-top:1px solid {t.rule};",
                    input {
                        "data-workspaces-name": "",
                        r#type: "text",
                        value: "{name}",
                        placeholder: "Save the current layout as…",
                        "aria-label": "Save the current layout as",
                        style: "flex:1; min-width:0; height:30px; padding:0 8px; border-radius:6px; border:1px solid {t.rule}; \
                                background:{t.desk}; color:{t.ink}; font-size:13px; outline:none;",
                        oninput: move |e| name.set(e.value()),
                        onkeydown: move |e| {
                            if e.key() == Key::Named(NamedKey::Enter) {
                                on_enter();
                            }
                        },
                    }
                    button {
                        "data-workspaces-save-new": "",
                        style: "height:30px; padding:0 12px; border-radius:6px; border:none; cursor:pointer; \
                                background:{t.accent}; color:{t.accent_ink}; font-size:13px;",
                        onclick: move |_| {
                            let mut save = save_new;
                            save();
                        },
                        "Save"
                    }
                }
            }
        }
    }
}

/// One saved layout and what can be done with it.
#[component]
fn SetRow(row: Row, dirty: bool) -> Element {
    let host = use_host();
    let t = *host.theme.read();
    let mut renaming = use_signal(|| None::<String>);
    let id = row.id.clone();
    let button = format!(
        "height:26px; padding:0 9px; border-radius:6px; border:1px solid {}; cursor:pointer; \
         background:transparent; color:{}; font-size:12px;",
        t.rule, t.ink
    );
    let slot = row
        .slot
        .map_or_else(|| "No key".to_owned(), |n| format!("⌘{n}"));
    let (load_id, save_id, rename_id, slot_id, end_id) =
        (id.clone(), id.clone(), id.clone(), id.clone(), id.clone());
    let name = row.name.clone();
    let weight = if row.active { 600 } else { 400 };
    let commit = move |typed: String| {
        renaming.set(None);
        let typed = typed.trim().to_owned();
        if !typed.is_empty() {
            host.write(
                control::rename,
                RenameArgs {
                    if_revision: None,
                    id: rename_id.clone(),
                    name: typed,
                },
            );
        }
    };
    rsx! {
        div {
            "data-workspace-row": "{id}",
            style: "display:flex; gap:6px; align-items:center; min-height:34px;",
            div { style: "flex:1; min-width:0; display:flex; gap:6px; align-items:center; font-size:13px;",
                match renaming() {
                    Some(typed) => rsx! {
                        input {
                            "data-workspace-rename-field": "{id}",
                            r#type: "text",
                            value: "{typed}",
                            style: "flex:1; min-width:0; height:26px; padding:0 6px; border-radius:6px; \
                                    border:1px solid {t.rule}; background:{t.desk}; color:{t.ink}; font-size:13px;",
                            oninput: move |e| renaming.set(Some(e.value())),
                            onkeydown: {
                                let mut commit = commit;
                                move |e: KeyboardEvent| {
                                    if e.key() == Key::Named(NamedKey::Enter) {
                                        let typed = renaming.peek().clone().unwrap_or_default();
                                        commit(typed);
                                    } else if e.key() == Key::Named(NamedKey::Escape) {
                                        renaming.set(None);
                                    }
                                }
                            },
                        }
                    },
                    None => rsx! {
                        span { style: "font-weight:{weight};", "{row.name}" }
                        if row.active {
                            span { style: "color:{t.ink_3}; font-size:11px;", if dirty { "active · changed" } else { "active" } }
                        }
                    },
                }
            }
            button {
                "data-workspace-load": "{id}",
                style: "{button}",
                onclick: move |_| {
                    host.write(
                        control::load,
                        LoadArgs {
                            if_revision: None,
                            id: Some(load_id.clone()),
                            name: None,
                            slot: None,
                        },
                    );
                },
                "Load"
            }
            button {
                "data-workspace-save": "{id}",
                style: "{button}",
                title: "Save the current layout over it",
                onclick: move |_| {
                    let name = host.screensets.peek().get(&save_id).map(|s| s.name.clone());
                    host.write(
                        control::save,
                        SaveArgs {
                            if_revision: None,
                            name,
                        },
                    );
                },
                "Save"
            }
            button {
                "data-workspace-rename": "{id}",
                style: "{button}",
                onclick: move |_| renaming.set(Some(name.clone())),
                "Rename"
            }
            button {
                "data-workspace-slot": "{id}",
                style: "{button}",
                title: "The key that loads it",
                onclick: move |_| {
                    host.write(
                        control::set_slot,
                        SlotArgs {
                            if_revision: None,
                            id: slot_id.clone(),
                            slot: next_slot(row.slot),
                        },
                    );
                },
                "{slot}"
            }
            if row.active && dirty {
                button {
                    "data-workspace-reset": "{id}",
                    style: "{button}",
                    title: "Back to its saved layout",
                    onclick: move |_| host.write(control::reset, control::RevisionArgs::default()),
                    "Reset"
                }
            }
            if !row.builtin {
                button {
                    "data-workspace-delete": "{id}",
                    style: "{button} color:{t.danger};",
                    onclick: move |_| {
                        host.write(
                            control::delete,
                            DeleteArgs {
                                if_revision: None,
                                id: end_id.clone(),
                            },
                        );
                    },
                    "Delete"
                }
            }
        }
    }
}
