//! The tag and property rows (Notebook Navigator's `TagTreeItem`,
//! `PropertyTreeItem`), their menu, the bars that ask for a name or a yes,
//! the search box, the banner, shortcut and recent-note rows over the
//! folders, and dragging notes and tags onto them (`useDragAndDrop`).
//!
//! A drag starts on a note's row or a tag's with the mouse pressed and
//! moved; Blitz sends the moves and the release to the row pressed, so it
//! finds the row under the pointer itself, from the navigation pane's
//! place on screen ([`Navigator`]'s `nav_box`) and the rows' offsets.

// `Navigator` is a `Copy` handle of signals, taken as the components take it.
#![allow(clippy::large_types_passed_by_value)]

use dioxus::prelude::*;
use lucide_dioxus::{
    ArrowDown, ArrowUp, BookmarkPlus, ChevronDown, ChevronRight, Equal, FileCode, FileText, Folder,
    Search, Tag, Tags, TextAlignStart, X,
};

use super::{Navigator, Open};
use crate::profiles::{Shortcut, ShortcutKind};
use crate::properties::ROOT;
use crate::selection::NavTarget;
use crate::state::{Drag, HideTarget, Mods, Outcome};
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
    // Every folder, tag and property but the virtual rows: a shortcut, and
    // (folders and tags) hiding it.
    let real = match &menu.target {
        NavTarget::Tag(t) => t != TAGGED && t != UNTAGGED,
        NavTarget::Property { key, .. } => key != ROOT,
        NavTarget::Folder(_) => true,
    };
    if real {
        let shortcut = Shortcut::to(&menu.target);
        let has = s.profile().shortcuts.iter().any(|x| x.same(&shortcut));
        items.push(if has {
            (
                "Remove from shortcuts".to_owned(),
                MenuAct::Shortcut(shortcut, false),
            )
        } else {
            (
                "Add to shortcuts".to_owned(),
                MenuAct::Shortcut(shortcut, true),
            )
        });
        match &menu.target {
            NavTarget::Folder(f) if !f.is_empty() => {
                items.push((
                    "Hide folder".to_owned(),
                    MenuAct::Hide(HideTarget::Folder(f.clone())),
                ));
            }
            NavTarget::Tag(t) => {
                items.push((
                    "Hide tag".to_owned(),
                    MenuAct::Hide(HideTarget::Tag(t.clone())),
                ));
            }
            _ => {}
        }
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
    /// A shortcut added (`true`) or taken out.
    Shortcut(Shortcut, bool),
    Hide(HideTarget),
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
        MenuAct::Shortcut(sc, add) => {
            nav.update(|s| {
                if add {
                    s.add_shortcut(sc, None);
                } else {
                    s.remove_shortcut(&sc);
                }
            });
            return;
        }
        MenuAct::Hide(what) => {
            nav.update(|s| s.hide(&what, false));
            nav.say("Hidden: show hidden items to see it again");
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

/// The search box over the list (`SearchInput`): what it types narrows
/// the list (the filter search, `crate::search`); ⌘/Ctrl-clicks on tags
/// and properties write to it. Escape closes and clears it; the bookmark
/// saves it as a shortcut.
#[component]
pub(super) fn SearchBar(nav: Navigator) -> Element {
    let s = nav.state.read();
    let text = s.search().text().to_owned();
    let open = s.search_open || !text.is_empty();
    let expression = s.search().is_expression();
    let full = s.settings().search_provider == crate::SearchProvider::FullText;
    drop(s);
    if !open {
        return rsx! {};
    }
    let placeholder = if full {
        "Search the notes' text, #tags, .properties, @dates"
    } else {
        "Search names, #tags, .properties, @dates, folder:"
    };
    let save = text.clone();
    rsx! {
        div { class: "nv-query nv-search", "data-testid": "nv-query", "data-query": "{text}",
            "data-expression": if expression { "true" } else { "false" },
            style: "flex:0 0 30px; display:flex; align-items:center; gap:6px; padding:0 8px; border-bottom:1px solid var(--nv-divider); \
                    color:var(--nv-muted); font-size:12px; white-space:nowrap; overflow:hidden;",
            Search { size: 12 }
            input { class: "nv-search-input", "data-testid": "nv-search-input", value: "{text}",
                placeholder,
                "aria-label": "Search",
                autofocus: true,
                style: "flex:1 1 auto; min-width:0; height:22px; padding:0 4px; border:none; outline:none; background:transparent; color:var(--nv-text); font-size:12px;",
                oninput: move |e| nav.set_search(&e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == Key::Named(NamedKey::Escape) {
                        nav.update(crate::NavigatorState::toggle_search);
                    }
                },
            }
            if !text.is_empty() {
                button { class: "nv-tool", "aria-label": "Save the search as a shortcut", "data-testid": "nv-search-save", style: "width:20px; height:20px;",
                    onclick: move |_| {
                        let added = nav.update(|s| s.add_shortcut(Shortcut::search(save.clone(), save.clone()), None));
                        nav.say(if added { "Saved to shortcuts" } else { "It is in the shortcuts" });
                    },
                    BookmarkPlus { size: 12 }
                }
            }
            button { class: "nv-tool", "aria-label": "Clear the search", style: "width:20px; height:20px;",
                onclick: move |_| nav.update(crate::NavigatorState::toggle_search),
                X { size: 12 }
            }
        }
    }
}

// ── Over the folders: the banner, the shortcuts, the recent notes ─────

/// The profile's banner (`NavigationBanner`): a vault image or a URL.
#[component]
pub(super) fn BannerRow(nav: Navigator, src: String, top: f64, height: f64) -> Element {
    let url = if src.contains("://") || src.starts_with("data:") {
        Some(src)
    } else {
        nav.source().resource_url(&src)
    };
    let Some(url) = url else {
        return rsx! {};
    };
    rsx! {
        div { class: "nv-banner", "data-testid": "nv-banner",
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; padding:4px 0; box-sizing:border-box;",
            img { src: "{url}", alt: "", style: "width:100%; height:100%; object-fit:cover; border-radius:8px; display:block;" }
        }
    }
}

/// The "Shortcuts" or "Recent notes" header, folding its rows.
#[component]
pub(super) fn SectionRow(
    nav: Navigator,
    id: String,
    name: String,
    expanded: bool,
    count: usize,
    top: f64,
    height: f64,
) -> Element {
    let toggle = id.clone();
    rsx! {
        div { class: "nv-row nv-section", role: "treeitem", "data-section": "{id}",
            "aria-expanded": if expanded { "true" } else { "false" },
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; display:flex; align-items:center; gap:4px; \
                    padding:0 8px 0 6px; border-radius:6px; white-space:nowrap; color:var(--nv-muted); font-size:11px; font-weight:600; \
                    text-transform:uppercase; letter-spacing:0.04em;",
            onclick: move |_| nav.update(|s| s.toggle_section(&toggle)),
            span { style: "flex:0 0 16px; display:flex; align-items:center; justify-content:center; color:var(--nv-faint);",
                if expanded { ChevronDown { size: 14 } } else { ChevronRight { size: 14 } }
            }
            span { style: "flex:1 1 auto;", "{name}" }
            span { class: "nv-count", style: "flex:0 0 auto; color:var(--nv-faint); font-weight:400;", "{count}" }
        }
    }
}

/// A shortcut: a click opens it; its number (1 to 9) is its command's;
/// reordering shows its arrows and a remove.
#[component]
pub(super) fn ShortcutRow(
    nav: Navigator,
    on_open: EventHandler<Open>,
    at: usize,
    shortcut: Shortcut,
    name: String,
    missing: bool,
    reorder: bool,
    top: f64,
    height: f64,
) -> Element {
    let badge = (at < 9).then(|| at.saturating_add(1).to_string());
    let open = shortcut.clone();
    let remove = shortcut.clone();
    let kind = shortcut.kind;
    let colour = if missing {
        "var(--nv-faint)"
    } else {
        "var(--nv-text)"
    };
    let deco = if missing { "line-through" } else { "none" };
    rsx! {
        div { class: "nv-row nv-shortcut", role: "treeitem", "data-shortcut": "{at}", "data-kind": "{kind.id()}",
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; display:flex; align-items:center; gap:6px; \
                    padding:0 8px 0 10px; border-radius:6px; white-space:nowrap; color:{colour};",
            onclick: move |_| match nav.update(|s| s.open_shortcut_entry(&open)) {
                Ok(out) => nav.act(out, on_open),
                Err(e) => nav.say(e),
            },
            span { style: "flex:0 0 auto; display:flex; color:var(--nv-faint);",
                match kind {
                    ShortcutKind::Note => rsx! { FileText { size: 15 } },
                    ShortcutKind::Folder => rsx! { Folder { size: 15 } },
                    ShortcutKind::Tag => rsx! { Tag { size: 15 } },
                    ShortcutKind::Property => rsx! { TextAlignStart { size: 15 } },
                    ShortcutKind::Search => rsx! { Search { size: 15 } },
                }
            }
            span { class: "nv-name", style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; text-decoration:{deco};", "{name}" }
            if reorder {
                button { class: "nv-tool", "aria-label": "Move {name} up", style: "width:20px; height:20px;",
                    onclick: move |e| { e.stop_propagation(); nav.update(|s| s.move_shortcut(at, at.saturating_sub(1))); },
                    ArrowUp { size: 13 }
                }
                button { class: "nv-tool", "aria-label": "Move {name} down", style: "width:20px; height:20px;",
                    onclick: move |e| { e.stop_propagation(); nav.update(|s| s.move_shortcut(at, at.saturating_add(1))); },
                    ArrowDown { size: 13 }
                }
                button { class: "nv-tool", "aria-label": "Remove {name} from shortcuts", style: "width:20px; height:20px;",
                    onclick: move |e| { e.stop_propagation(); nav.update(|s| s.remove_shortcut(&remove)); },
                    X { size: 13 }
                }
            } else if let Some(b) = badge {
                span { class: "nv-count nv-shortcut-badge", style: "flex:0 0 auto; color:var(--nv-faint); font-size:11px;", "{b}" }
            }
        }
    }
}

/// A recent note: a click opens it.
#[component]
pub(super) fn RecentRow(
    nav: Navigator,
    on_open: EventHandler<Open>,
    path: String,
    name: String,
    active: bool,
    top: f64,
    height: f64,
) -> Element {
    let class = if active {
        "nv-row nv-recent nv-selected"
    } else {
        "nv-row nv-recent"
    };
    let p = path.clone();
    rsx! {
        div { class, role: "treeitem", "data-recent": "{path}",
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; display:flex; align-items:center; gap:6px; \
                    padding:0 8px 0 10px; border-radius:6px; white-space:nowrap;",
            onclick: move |_| {
                let out = nav.state.peek().open_recent(&p);
                nav.act(out, on_open);
            },
            span { style: "flex:0 0 auto; display:flex; color:var(--nv-faint);", FileText { size: 15 } }
            span { class: "nv-name", style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis;", "{name}" }
        }
    }
}

/// The modifiers as the model takes them, for a click on a pill.
pub(super) fn click_mods(e: &MouseEvent) -> Mods {
    super::panes::mods(e.modifiers())
}
