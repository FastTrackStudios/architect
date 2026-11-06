//! The tag and property rows (Notebook Navigator's `TagTreeItem`,
//! `PropertyTreeItem`), their menu, the bars that ask for a name or a yes,
//! the search a ⌘/Ctrl-click built, and dragging notes and tags onto them
//! (`useDragAndDrop`).
//!
//! A drag starts on a note's row or a tag's with the mouse pressed and
//! moved; Blitz sends the moves and the release to the row pressed, so it
//! finds the row under the pointer itself, from the navigation pane's
//! place on screen ([`Navigator`]'s `nav_box`) and the rows' offsets.

// `Navigator` is a `Copy` handle of signals, taken as the components take it.
#![allow(clippy::large_types_passed_by_value)]

use dioxus::prelude::*;
use lucide_dioxus::{
    ArrowDown, ArrowUp, ChevronDown, ChevronRight, Equal, FileCode, Search, Tag, Tags,
    TextAlignStart, X,
};

use super::{Navigator, Open};
use crate::properties::ROOT;
use crate::query::Query;
use crate::selection::NavTarget;
use crate::state::{Drag, Mods, Outcome};
use crate::tags::{TAGGED, UNTAGGED};

/// How far a press moves before it is a drag.
const DRAG_SLOP: f64 = 4.0;

/// A drag under way: what, from where, and the row it is over.
#[derive(Debug, Clone, PartialEq)]
pub struct DragNow {
    pub what: Drag,
    pub from: (f64, f64),
    /// Moved past the slop: a drag, not a click.
    pub active: bool,
    /// The row key under the pointer, when it takes a drop.
    pub over: Option<String>,
}

/// A row's menu, open.
#[derive(Debug, Clone, PartialEq)]
pub struct RowMenu {
    pub target: NavTarget,
    pub top: f64,
}

/// A name asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptKind {
    /// A tag's new name.
    RenameTag(String),
    /// A tag for these notes.
    AddTag(Vec<String>),
    /// A value of `key` for these notes.
    SetValue { key: String, paths: Vec<String> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub title: String,
    pub value: String,
    pub kind: PromptKind,
}

/// The icon a row draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowIcon {
    AllTags,
    Tag,
    Untagged,
    AllProperties,
    Key,
    Value,
}

/// One tag or property row.
#[component]
pub(super) fn ItemRow(
    nav: Navigator,
    on_open: EventHandler<Open>,
    target: NavTarget,
    row_key: String,
    name: String,
    level: u16,
    has_children: bool,
    expanded: bool,
    count: String,
    icon: RowIcon,
    colour: Option<String>,
    selected: bool,
    top: f64,
    height: f64,
    indent: f64,
    /// Up and down arrows: a root tag or a property key, while reordering.
    reorder: bool,
    /// Under a drag that it takes.
    drop_here: bool,
) -> Element {
    let pad = f64::from(level).mul_add(indent, 6.0);
    let class = match (selected, drop_here) {
        (_, true) => "nv-row nv-drop-target",
        (true, false) => "nv-row nv-selected",
        (false, false) => "nv-row",
    };
    let weight = if matches!(icon, RowIcon::AllTags | RowIcon::AllProperties) {
        600
    } else {
        400
    };
    let name_colour = colour
        .as_deref()
        .map_or_else(String::new, |c| format!("color:{c};"));
    let (kind_attr, id_attr) = match &target {
        NavTarget::Tag(t) => ("tag", t.clone()),
        NavTarget::Property { key, value } => (
            "property",
            value
                .as_ref()
                .map_or_else(|| key.clone(), |v| format!("{key}={v}")),
        ),
        NavTarget::Folder(f) => ("folder", f.clone()),
    };
    let draggable = match (&target, icon) {
        (NavTarget::Tag(t), RowIcon::Tag) => Some(t.clone()),
        _ => None,
    };
    let (t1, t2, t3, t4) = (
        target.clone(),
        target.clone(),
        target.clone(),
        target.clone(),
    );
    let menu_target = target;
    rsx! {
        div {
            class,
            role: "treeitem",
            "data-drop-zone": "{kind_attr}",
            "data-tag": (kind_attr == "tag").then(|| id_attr.clone()),
            "data-property": (kind_attr == "property").then(|| id_attr.clone()),
            "data-row": "{row_key}",
            "aria-selected": if selected { "true" } else { "false" },
            "aria-expanded": if has_children { if expanded { "true" } else { "false" } } else { "" },
            "aria-level": "{level}",
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; display:flex; align-items:center; \
                    gap:4px; padding-left:{pad}px; padding-right:8px; border-radius:6px; white-space:nowrap;",
            onclick: move |e: MouseEvent| {
                if nav.dragged.cloned() {
                    return;
                }
                let m = super::panes::mods(e.modifiers());
                match &t1 {
                    NavTarget::Tag(t) => nav.update(|s| s.click_tag(t, m)),
                    NavTarget::Property { key, value } => {
                        nav.update(|s| s.click_property(key, value.as_deref(), m));
                    }
                    NavTarget::Folder(f) => {
                        let out = nav.update(|s| s.click_folder(f));
                        nav.act(out, on_open);
                    }
                }
            },
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let mut menu = nav.menu;
                menu.set(Some(RowMenu { target: menu_target.clone(), top: top + height }));
            },
            onpointerdown: move |e: PointerEvent| {
                if let Some(tag) = &draggable {
                    start_drag(nav, Drag::Tag(tag.clone()), &e);
                }
            },
            onpointermove: move |e: PointerEvent| drag_move(nav, &e),
            onpointerup: move |_| drag_end(nav, on_open),
            div {
                class: "nv-chevron",
                "aria-label": if expanded { "Collapse" } else { "Expand" },
                style: "flex:0 0 16px; height:16px; display:flex; align-items:center; justify-content:center; color:var(--nv-faint);",
                onclick: move |e| {
                    e.stop_propagation();
                    if has_children {
                        nav.update(|s| s.toggle_nav(&t2));
                    }
                },
                if has_children {
                    if expanded { ChevronDown { size: 14 } } else { ChevronRight { size: 14 } }
                }
            }
            span { style: "flex:0 0 auto; display:flex; color:var(--nv-faint); {name_colour}",
                match icon {
                    RowIcon::AllTags => rsx! { Tags { size: 15 } },
                    RowIcon::Tag => rsx! { Tag { size: 14 } },
                    RowIcon::Untagged => rsx! { span { style: "display:flex; opacity:0.5;", Tag { size: 14 } } },
                    RowIcon::AllProperties => rsx! { FileCode { size: 15 } },
                    RowIcon::Key => rsx! { TextAlignStart { size: 14 } },
                    RowIcon::Value => rsx! { Equal { size: 14 } },
                }
            }
            span { class: "nv-name",
                style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; font-weight:{weight}; {name_colour}",
                "{name}"
            }
            if reorder {
                button { class: "nv-tool", "aria-label": "Move {name} up", style: "width:20px; height:20px;",
                    onclick: move |e| { e.stop_propagation(); reorder_by(nav, &t3, -1); },
                    ArrowUp { size: 13 }
                }
                button { class: "nv-tool", "aria-label": "Move {name} down", style: "width:20px; height:20px;",
                    onclick: move |e| { e.stop_propagation(); reorder_by(nav, &t4, 1); },
                    ArrowDown { size: 13 }
                }
            } else if !count.is_empty() {
                span { class: "nv-count", style: "flex:0 0 auto; color:var(--nv-faint); font-size:11px;", "{count}" }
            }
        }
    }
}

fn reorder_by(nav: Navigator, target: &NavTarget, by: isize) {
    match target {
        NavTarget::Tag(t) => nav.update(|s| s.move_tag_root(t, by)),
        NavTarget::Property { key, .. } => nav.update(|s| s.move_property_key(key, by)),
        NavTarget::Folder(_) => {}
    }
}

// ── Dragging ───────────────────────────────────────────────────────────

/// A mouse press on a note's row or a tag's: a drag, once it moves.
pub(super) fn start_drag(nav: Navigator, what: Drag, e: &PointerEvent) {
    // A finger scrolls the pane (Notebook Navigator drags on a desk alone).
    if e.data().pointer_type() != "mouse" {
        return;
    }
    let p = e.data().client_coordinates();
    let mut drag = nav.drag;
    drag.set(Some(DragNow {
        what,
        from: (p.x, p.y),
        active: false,
        over: None,
    }));
}

/// The pointer moved with a drag pressed: past the slop it is one, and
/// the row under it found.
pub(super) fn drag_move(nav: Navigator, e: &PointerEvent) {
    let Some(mut now) = nav.drag.peek().clone() else {
        return;
    };
    let p = e.data().client_coordinates();
    if !now.active {
        if (p.x - now.from.0).hypot(p.y - now.from.1) < DRAG_SLOP {
            return;
        }
        now.active = true;
        let mut dragged = nav.dragged;
        dragged.set(true);
    }
    let over = drop_row(nav, p.x, p.y).map(|(key, _)| key);
    if over != now.over || !nav.drag.peek().as_ref().is_some_and(|d| d.active) {
        now.over = over;
        let mut drag = nav.drag;
        drag.set(Some(now));
    }
}

/// The row under the client point `x`, `y` that takes a drop: its key and
/// what it selects.
fn drop_row(nav: Navigator, x: f64, y: f64) -> Option<(String, NavTarget)> {
    let (bx, by, bw, bh) = (*nav.nav_box.peek())?;
    if x < bx || x > bx + bw || y < by || y > by + bh {
        return None;
    }
    let s = nav.state.peek();
    let row = s.nav_row_at(y - by)?;
    let target = row.target()?;
    matches!(target, NavTarget::Tag(_) | NavTarget::Property { .. })
        .then(|| (row.key.clone(), target))
}

/// The press let go: a drag ends on the row under it.
pub(super) fn drag_end(nav: Navigator, on_open: EventHandler<Open>) {
    let mut drag = nav.drag;
    let Some(now) = drag.take() else { return };
    if !now.active {
        return;
    }
    let Some(key) = now.over else { return };
    let target = nav
        .state
        .peek()
        .nav_rows()
        .iter()
        .find(|r| r.key == key)
        .and_then(crate::tree::NavRow::target);
    let Some(target) = target else { return };
    let out = nav.state.peek().drop_on(&now.what, &target);
    match (out, &now.what) {
        // A tag moved is a rename: asked first, as Notebook Navigator
        // opens its rename dialog filled in.
        (Ok(out), Drag::Tag(src)) => {
            let to = match &out {
                Outcome::Write { ops, .. } => match ops.first() {
                    Some(crate::source::FileOp::RenameTag { to, .. }) => to.clone(),
                    _ => String::new(),
                },
                _ => String::new(),
            };
            let mut ask = nav.ask;
            ask.set(Some((
                format!("Rename #{src} to #{to}, in every note?"),
                out,
            )));
        }
        (Ok(out), Drag::Notes(_)) => nav.act(Some(out), on_open),
        (Err(e), _) => nav.say(e),
    }
}

// ── The row menu ───────────────────────────────────────────────────────

/// A tag's or a property's menu (`buildTagMenu`, the property menu).
#[component]
pub(super) fn Menu(nav: Navigator, on_open: EventHandler<Open>, menu: RowMenu) -> Element {
    let s = nav.state.read();
    let paths: Vec<String> = s.selection().files().iter().cloned().collect();
    let n = paths.len();
    let shown_tag = |t: &str| {
        s.tag_tree()
            .get(t)
            .map_or_else(|| t.to_owned(), |node| node.display.clone())
    };
    let mut items: Vec<(String, MenuAct)> = Vec::new();
    match &menu.target {
        NavTarget::Tag(t) if t == TAGGED => {
            items.push(("Reset tag order".to_owned(), MenuAct::ResetTagOrder));
        }
        NavTarget::Tag(t) if t == UNTAGGED => {
            if n > 0 {
                items.push((
                    format!("Clear the tags of {n} selected"),
                    MenuAct::Clear(paths),
                ));
            }
        }
        NavTarget::Tag(t) => {
            let t = shown_tag(t);
            items.push((format!("New note with #{t}"), MenuAct::NewNote(t.clone())));
            if n > 0 {
                items.push((
                    format!("Add #{t} to {n} selected"),
                    MenuAct::Add(paths.clone(), t.clone()),
                ));
                items.push((
                    format!("Remove #{t} from {n} selected"),
                    MenuAct::Remove(paths, t.clone()),
                ));
            }
            items.push(("Rename tag…".to_owned(), MenuAct::Rename(t.clone())));
            items.push((format!("Delete #{t}"), MenuAct::Delete(t)));
        }
        NavTarget::Property { key, .. } if key == ROOT => {}
        NavTarget::Property { key, value } => {
            let name = s
                .property_tree()
                .key(key)
                .map_or_else(|| key.clone(), |k| k.name.clone());
            if n > 0 {
                let label = value.as_ref().map_or_else(
                    || format!("Set {name} on {n} selected"),
                    |v| {
                        let shown = s
                            .property_tree()
                            .value(key, v)
                            .map_or_else(|| v.clone(), |node| node.name.clone());
                        format!("Set {name}: {shown} on {n} selected")
                    },
                );
                items.push((
                    label,
                    MenuAct::SetProperty(paths.clone(), key.clone(), value.clone()),
                ));
                items.push((
                    format!("Set {name} on {n} selected to…"),
                    MenuAct::SetValue(paths, key.clone()),
                ));
            }
        }
        NavTarget::Folder(_) => {}
    }
    if n == 0 && matches!(menu.target, NavTarget::Property { .. } | NavTarget::Tag(_)) {
        items.push((
            "Select notes to tag them or set properties".to_owned(),
            MenuAct::Nothing,
        ));
    }
    drop(s);
    let top = menu.top;
    let mut open = nav.menu;
    rsx! {
        // A press anywhere else closes it.
        div { class: "nv-menu-backdrop", style: "position:absolute; inset:0; z-index:9;",
            onclick: move |_| open.set(None),
            oncontextmenu: move |e: MouseEvent| { e.prevent_default(); open.set(None); },
        }
        div { class: "nv-menu nv-row-menu", role: "menu",
            style: "position:absolute; left:12px; right:12px; top:{top}px; z-index:10; padding:4px; border-radius:8px; \
                    background:var(--nv-background); border:1px solid var(--nv-divider); display:flex; flex-direction:column;",
            for (label, act) in items {
                button { class: "nv-menu-row", role: "menuitem", "data-menu-item": "{label}",
                    style: "text-align:left; height:26px; padding:0 8px; border:none; border-radius:5px; background:transparent; color:var(--nv-text); cursor:pointer; font-size:12px;",
                    onclick: move |_| {
                        open.set(None);
                        menu_act(nav, on_open, act.clone());
                    },
                    "{label}"
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum MenuAct {
    NewNote(String),
    Add(Vec<String>, String),
    Remove(Vec<String>, String),
    Clear(Vec<String>),
    Rename(String),
    Delete(String),
    ResetTagOrder,
    SetProperty(Vec<String>, String, Option<String>),
    SetValue(Vec<String>, String),
    Nothing,
}

fn menu_act(nav: Navigator, on_open: EventHandler<Open>, act: MenuAct) {
    let out = match act {
        MenuAct::NewNote(t) => nav.state.peek().new_note_in_tag(&t),
        MenuAct::Add(paths, t) => nav.state.peek().add_tag(&paths, &t),
        MenuAct::Remove(paths, t) => nav.state.peek().remove_tag(&paths, &t),
        MenuAct::Clear(paths) => nav.state.peek().clear_tags(&paths),
        MenuAct::SetProperty(paths, key, value) => {
            nav.state
                .peek()
                .set_property(&paths, &key, value.as_deref())
        }
        MenuAct::Rename(t) => {
            let mut prompt = nav.prompt;
            prompt.set(Some(Prompt {
                title: format!("Rename #{t}"),
                value: t.clone(),
                kind: PromptKind::RenameTag(t),
            }));
            return;
        }
        MenuAct::SetValue(paths, key) => {
            let mut prompt = nav.prompt;
            prompt.set(Some(Prompt {
                title: format!("Set {key} on {} notes", paths.len()),
                value: String::new(),
                kind: PromptKind::SetValue { key, paths },
            }));
            return;
        }
        MenuAct::Delete(t) => match nav.state.peek().delete_tag(&t) {
            Ok(out) => {
                let mut ask = nav.ask;
                ask.set(Some((
                    format!("Delete #{t} and the tags below it from every note?"),
                    out,
                )));
                return;
            }
            Err(e) => Err(e),
        },
        MenuAct::ResetTagOrder => {
            nav.update(crate::NavigatorState::reset_tag_order);
            return;
        }
        MenuAct::Nothing => return,
    };
    match out {
        Ok(out) => nav.act(Some(out), on_open),
        Err(e) => nav.say(e),
    }
}

// ── Asking ─────────────────────────────────────────────────────────────

/// A bar over the navigator's foot asking for a name.
#[component]
pub(super) fn PromptBar(nav: Navigator, on_open: EventHandler<Open>, prompt: Prompt) -> Element {
    let mut value = use_signal(|| prompt.value.clone());
    let mut open = nav.prompt;
    let kind = prompt.kind.clone();
    let submit = move || {
        let text = value.peek().trim().to_owned();
        let out = match &kind {
            PromptKind::RenameTag(from) => nav.state.peek().rename_tag(from, &text),
            PromptKind::AddTag(paths) => nav.state.peek().add_tag(paths, &text),
            PromptKind::SetValue { key, paths } => {
                nav.state.peek().set_property(paths, key, Some(&text))
            }
        };
        match out {
            Ok(out) => {
                open.set(None);
                nav.act(Some(out), on_open);
            }
            Err(e) => nav.say(e),
        }
    };
    let mut on_enter = submit.clone();
    let mut on_ok = submit;
    rsx! {
        div { class: "nv-confirm nv-prompt", role: "dialog", "aria-label": "{prompt.title}",
            style: "position:absolute; left:8px; right:8px; bottom:8px; z-index:20; padding:10px; border-radius:10px; \
                    background:var(--nv-background); border:1px solid var(--nv-divider); display:flex; flex-direction:column; gap:8px;",
            div { style: "font-weight:600;", "{prompt.title}" }
            input { class: "nv-prompt-input", "data-testid": "nv-prompt-input", value: "{value}",
                style: "height:26px; padding:0 8px; border-radius:6px; border:1px solid var(--nv-divider); background:transparent; color:var(--nv-text);",
                oninput: move |e| value.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == Key::Named(NamedKey::Enter) {
                        on_enter();
                    }
                },
            }
            div { style: "display:flex; justify-content:flex-end; gap:6px;",
                button { class: "nv-confirm-cancel", "data-testid": "nv-prompt-cancel",
                    style: "height:26px; padding:0 10px; border-radius:6px; border:1px solid var(--nv-divider); background:transparent; color:var(--nv-text); cursor:pointer;",
                    onclick: move |_| open.set(None),
                    "Cancel"
                }
                button { class: "nv-confirm-create", "data-testid": "nv-prompt-ok",
                    style: "height:26px; padding:0 10px; border-radius:6px; border:none; background:var(--nv-open); color:var(--nv-background); cursor:pointer; font-weight:600;",
                    onclick: move |_| on_ok(),
                    "OK"
                }
            }
        }
    }
}

/// A bar asking a yes before a write that changes many notes.
#[component]
pub(super) fn AskBar(
    nav: Navigator,
    on_open: EventHandler<Open>,
    text: String,
    out: Outcome,
) -> Element {
    let mut ask = nav.ask;
    rsx! {
        div { class: "nv-confirm nv-ask", role: "alertdialog", "aria-label": "{text}",
            style: "position:absolute; left:8px; right:8px; bottom:8px; z-index:20; padding:10px; border-radius:10px; \
                    background:var(--nv-background); border:1px solid var(--nv-divider); display:flex; flex-direction:column; gap:8px;",
            div { style: "overflow-wrap:anywhere;", "{text}" }
            div { style: "display:flex; justify-content:flex-end; gap:6px;",
                button { class: "nv-confirm-cancel", "data-testid": "nv-ask-cancel",
                    style: "height:26px; padding:0 10px; border-radius:6px; border:1px solid var(--nv-divider); background:transparent; color:var(--nv-text); cursor:pointer;",
                    onclick: move |_| ask.set(None),
                    "Cancel"
                }
                button { class: "nv-confirm-create", "data-testid": "nv-ask-ok",
                    style: "height:26px; padding:0 10px; border-radius:6px; border:none; background:var(--nv-open); color:var(--nv-background); cursor:pointer; font-weight:600;",
                    onclick: move |_| {
                        ask.set(None);
                        nav.act(Some(out.clone()), on_open);
                    },
                    "OK"
                }
            }
        }
    }
}

/// The search a ⌘/Ctrl-click built, over the list, with a button that
/// clears it.
#[component]
pub(super) fn QueryBar(nav: Navigator) -> Element {
    let text = nav.state.read().query().text();
    if text.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "nv-query", "data-testid": "nv-query", "data-query": "{text}",
            style: "flex:0 0 26px; display:flex; align-items:center; gap:6px; padding:0 10px; border-bottom:1px solid var(--nv-divider); \
                    color:var(--nv-muted); font-size:12px; white-space:nowrap; overflow:hidden;",
            Search { size: 12 }
            span { style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis;", "{text}" }
            button { class: "nv-tool", "aria-label": "Clear the search", style: "width:20px; height:20px;",
                onclick: move |_| nav.update(|s| s.set_query(Query::default())),
                X { size: 12 }
            }
        }
    }
}

/// The modifiers as the model takes them, for a click on a pill.
pub(super) fn click_mods(e: &MouseEvent) -> Mods {
    super::panes::mods(e.modifiers())
}
