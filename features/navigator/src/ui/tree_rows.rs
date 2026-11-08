//! The tag and property rows (Notebook Navigator's `TagTreeItem`,
//! `PropertyTreeItem`), their menu, the bars that ask for a name or a yes,
//! the search box, the banner, shortcut and recent-note rows over the
//! folders, and dragging notes and tags onto them (`useDragAndDrop`).
//!
//! A drag starts on a note's, a folder's, a tag's or a shortcut's row with
//! the mouse pressed and moved; Blitz sends the moves and the release to
//! the row pressed, so it finds the row under the pointer itself, from the
//! navigation pane's place on screen ([`Navigator`]'s `nav_box`) and the
//! rows' offsets. Held over a closed folder, the folder opens
//! (spring-loaded folders); the copy key (⌥ on a Mac, Ctrl elsewhere)
//! makes a drop on a folder a copy.

// `Navigator` is a `Copy` handle of signals, taken as the components take it.
#![allow(clippy::large_types_passed_by_value)]

use dioxus::prelude::*;
use lucide_dioxus::{
    ArrowDown, ArrowUp, BookmarkPlus, ChevronDown, ChevronRight, Equal, FileCode, FileText, Folder,
    Search, Tag, Tags, TextAlignStart, X,
};

use super::{Ask, AskThen, Navigator, Open, Renaming};
use crate::profiles::{Shortcut, ShortcutKind};
use crate::properties::ROOT;
use crate::selection::NavTarget;
use crate::state::{
    Drag, HideTarget, MergeSeparator, Mods, NewKind, NewNote, Outcome, Paint, check_name,
};
use crate::tags::{TAGGED, UNTAGGED};
use crate::tree::{NavRowKind, SHORTCUTS};

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
    /// The copy key held at the last move.
    pub copy: bool,
    /// Folders opened under it so far: the first waits longer.
    pub springs: u32,
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
    /// A property key's new name.
    RenameKey(String),
    /// A value's new name, of a key.
    RenameValue { key: String, value: String },
    /// A new folder's name, in this one.
    NewFolder(String),
    /// A new note's name, in this folder, of this kind.
    NewNamed { folder: String, kind: NewKind },
    /// The folder these files go to.
    MoveTo(Vec<String>),
    /// The folder this folder goes into.
    MoveFolderTo(String),
    /// The template a note made from in this folder takes.
    TemplateFor(String),
    /// A folder's template (a vault path; empty, none).
    FolderTemplate(String),
    /// The merged note's name, for these notes.
    Merge(Vec<String>),
    /// A template's prompt, `label`, for this request.
    Template { req: Box<NewNote>, label: String },
    /// An icon, a colour or a background for a target (`folder:…`,
    /// `tag:…`, `property:…`, `file:…`); empty takes it off.
    Paint { what: Paint, target: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub title: String,
    pub value: String,
    pub kind: PromptKind,
}

impl Prompt {
    /// Asks `target`'s icon, colour or background.
    #[must_use]
    pub fn paint(what: Paint, target: String) -> Self {
        Self {
            title: paint_title(what).to_owned(),
            value: String::new(),
            kind: PromptKind::Paint { what, target },
        }
    }

    /// Asks the folder `paths` move to.
    #[must_use]
    pub fn move_to(paths: Vec<String>) -> Self {
        Self {
            title: "Move to folder (a vault path; empty, the root)".to_owned(),
            value: String::new(),
            kind: PromptKind::MoveTo(paths),
        }
    }
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

/// A tag's or a property's own kind of icon.
fn row_icon(icon: RowIcon) -> Element {
    match icon {
        RowIcon::AllTags => rsx! { Tags { size: 15 } },
        RowIcon::Tag => rsx! { Tag { size: 14 } },
        RowIcon::Untagged => {
            rsx! { span { style: "display:flex; opacity:0.5;", Tag { size: 14 } } }
        }
        RowIcon::AllProperties => rsx! { FileCode { size: 15 } },
        RowIcon::Key => rsx! { TextAlignStart { size: 14 } },
        RowIcon::Value => rsx! { Equal { size: 14 } },
    }
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
    /// A tag the search takes in or leaves out.
    search: Option<crate::SearchMatch>,
    /// Its own icon (`tagIcons`, `propertyIcons`) and background.
    custom_icon: Option<crate::Icon>,
    background: Option<String>,
    /// The colour paints the icon alone.
    icon_only: bool,
) -> Element {
    let deck_rows = use_deck_rows();
    let deck_rows_down = deck_rows.clone();
    let target_down = target.clone();
    let pad = f64::from(level).mul_add(indent, 6.0);
    let name_flex = name_flex(nav, !count.is_empty() && !reorder);
    let class = match (selected, drop_here) {
        (_, true) => "nv-row nv-drop-target",
        (true, false) => "nv-row nv-selected",
        (false, false) => "nv-row",
    };
    let name_class = match search {
        Some(crate::SearchMatch::Include) => "nv-name nv-search-include",
        Some(crate::SearchMatch::Exclude) => "nv-name nv-search-exclude",
        None => "nv-name",
    };
    let weight = if matches!(icon, RowIcon::AllTags | RowIcon::AllProperties) {
        600
    } else {
        400
    };
    let name_colour = colour
        .as_deref()
        .filter(|_| !icon_only)
        .map_or_else(String::new, |c| format!("color:{c};"));
    let bg = background
        .filter(|_| !selected && !drop_here)
        .map_or_else(String::new, |b| format!("background:{b};"));
    let icon_colour = colour
        .clone()
        .unwrap_or_else(|| "var(--nv-faint)".to_owned());
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
                    gap:4px; padding-left:{pad}px; padding-right:8px; border-radius:6px; white-space:nowrap; {bg}",
            onclick: move |e: MouseEvent| {
                if nav.dragged.cloned() {
                    return;
                }
                let m = super::panes::mods(e.modifiers());
                match &t1 {
                    NavTarget::Tag(t) => {
                        nav.update(|s| s.click_tag(t, m));
                        if !m.command {
                            nav.act(nav.state.peek().first_note(), on_open);
                        }
                    }
                    NavTarget::Property { key, value } => {
                        nav.update(|s| s.click_property(key, value.as_deref(), m));
                        if !m.command {
                            nav.act(nav.state.peek().first_note(), on_open);
                        }
                    }
                    NavTarget::Folder(f) if m.command && deck_row(deck_rows.as_ref(), f) => {
                        open_deck(f, on_open);
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
                if let NavTarget::Folder(f) = &target_down {
                    if middle_click(&e, deck_rows_down.as_ref(), f, on_open) {
                        return;
                    }
                }
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
            span { style: "flex:0 0 auto; display:flex; color:{icon_colour};",
                if let Some(i) = custom_icon {
                    super::icon::IconView { nav, icon: i, size: 14, colour }
                } else {
                    {row_icon(icon)}
                }
            }
            span { class: name_class,
                style: "flex:{name_flex}; min-width:0; overflow:hidden; text-overflow:ellipsis; font-weight:{weight}; {name_colour}",
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
                Leader { nav }
                span { class: "nv-count", style: "flex:0 0 auto; color:var(--nv-faint); font-size:11px;", "{count}" }
            }
            Guides { nav, level, indent }
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

/// A mouse press on a note's, a folder's, a tag's or a shortcut's row: a
/// drag, once it moves.
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
        copy: false,
        springs: 0,
    }));
}

/// The copy key: ⌥ on a Mac, Ctrl elsewhere (`getVaultItemFolderDropEffect`).
fn copy_key(e: &PointerEvent) -> bool {
    let m = e.modifiers();
    if cfg!(target_os = "macos") {
        m.alt()
    } else {
        m.ctrl()
    }
}

/// The pointer moved with a drag pressed: past the slop it is one, and
/// the row under it found; a closed folder under it opens after a while.
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
    let over = drop_row(nav, &now.what, p.x, p.y);
    let copy = copy_key(e);
    let was_active = nav.drag.peek().as_ref().is_some_and(|d| d.active);
    if over != now.over || copy != now.copy || !was_active {
        let entered = over != now.over;
        now.over.clone_from(&over);
        now.copy = copy;
        let springs = now.springs;
        let mut drag = nav.drag;
        drag.set(Some(now));
        if entered && let Some(key) = over {
            spring(nav, key, springs);
        }
    }
}

/// Opens the closed folder at row `key` if the drag is still over it after
/// the settings' delay (`springLoadedFolders`).
fn spring(nav: Navigator, key: String, springs: u32) {
    let (on, first, next) = {
        let s = nav.state.peek();
        let st = s.settings();
        (
            st.spring_loaded_folders,
            st.spring_loaded_delay_ms,
            st.spring_loaded_next_delay_ms,
        )
    };
    // A collapsed folder, or (`scheduleTagAutoExpand`) a collapsed tag.
    let target = nav
        .state
        .peek()
        .nav_rows()
        .iter()
        .find(|r| r.key == key)
        .and_then(|r| match &r.kind {
            NavRowKind::Folder {
                path,
                has_children: true,
                expanded: false,
                ..
            } => Some(NavTarget::Folder(path.clone())),
            NavRowKind::Tag {
                tag,
                has_children: true,
                expanded: false,
                ..
            } => Some(NavTarget::Tag(tag.clone())),
            _ => None,
        });
    let Some(target) = target.filter(|_| on) else {
        return;
    };
    let wait = if springs == 0 { first } else { next };
    spawn(async move {
        futures_timer::Delay::new(std::time::Duration::from_millis(u64::from(wait))).await;
        let still = nav
            .drag
            .peek()
            .as_ref()
            .is_some_and(|d| d.active && d.over.as_deref() == Some(key.as_str()));
        if still && nav.update(|s| s.spring_open(&target)) {
            let mut drag = nav.drag;
            if let Some(d) = drag.write().as_mut() {
                d.springs = d.springs.saturating_add(1);
            }
        }
    });
}

/// The row under the client point `x`, `y` that takes `what` dropped: its
/// key. Folders take notes and folders; tags and properties take notes
/// and tags; the shortcuts take anything, a shortcut only them.
fn drop_row(nav: Navigator, what: &Drag, x: f64, y: f64) -> Option<String> {
    let (bx, by, bw, bh) = (*nav.nav_box.peek())?;
    if x < bx || x > bx + bw || y < by || y > by + bh {
        return None;
    }
    let zoom = nav.zoom();
    let s = nav.state.peek();
    let row = s.nav_row_at((y - by) / zoom)?;
    let takes = match (&row.kind, what) {
        (NavRowKind::Section { id, .. }, _) => id == SHORTCUTS,
        (NavRowKind::Shortcut { .. }, _)
        | (NavRowKind::Folder { .. }, Drag::Notes(_))
        | (NavRowKind::Tag { .. } | NavRowKind::Property { .. }, Drag::Notes(_) | Drag::Tag(_)) => {
            true
        }
        (NavRowKind::Folder { path, .. }, Drag::Folder(src)) => {
            path != src && !crate::source::is_within(path, src)
        }
        _ => false,
    };
    takes.then(|| row.key.clone())
}

/// The press let go: a drag ends on the row under it.
pub(super) fn drag_end(nav: Navigator, on_open: EventHandler<Open>) {
    let mut drag = nav.drag;
    let Some(now) = drag.take() else { return };
    if !now.active {
        return;
    }
    let Some(key) = now.over else { return };
    let row = nav
        .state
        .peek()
        .nav_rows()
        .iter()
        .find(|r| r.key == key)
        .cloned();
    let Some(row) = row else { return };
    // Among the shortcuts: moved or added there.
    let at = match &row.kind {
        NavRowKind::Shortcut { at, .. } => Some(*at),
        NavRowKind::Section { id, .. } if id == SHORTCUTS => Some(0),
        _ => None,
    };
    if let Some(at) = at {
        if nav.update(|s| s.drop_on_shortcuts(&now.what, at))
            && !matches!(now.what, Drag::Shortcut(_))
        {
            nav.say("Added to shortcuts");
        }
        return;
    }
    let Some(target) = row.target() else { return };
    let out = nav.state.peek().drop_on(&now.what, &target, now.copy);
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
            nav.ask(
                format!("Rename #{src} to #{to}, in every note?"),
                "OK",
                AskThen::Act(out),
            );
        }
        (Ok(out), _) => nav.act(Some(out), on_open),
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
            // A nested tag moves among its siblings (`tagChildOrder`).
            if t.contains('/') {
                items.push((
                    "Move up".to_owned(),
                    MenuAct::MoveChild(format!("tag:{t}"), -1),
                ));
                items.push((
                    "Move down".to_owned(),
                    MenuAct::MoveChild(format!("tag:{t}"), 1),
                ));
            }
            items.push((
                "Reset order of tags below".to_owned(),
                MenuAct::ResetChildOrder(format!("tag:{t}")),
            ));
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
            // `buildPropertyMenu`: the key's rename and delete, and (ours)
            // a value's rename.
            match value {
                None => {
                    items.push((
                        "Reset order of values".to_owned(),
                        MenuAct::ResetChildOrder(format!("property:{key}")),
                    ));
                    items.push((format!("Rename {name}…"), MenuAct::RenameKey(key.clone())));
                    items.push((
                        format!("Delete {name} from every note"),
                        MenuAct::DeleteKey(key.clone()),
                    ));
                }
                Some(v) => {
                    // A value moves among the key's (`propertyChildOrder`).
                    let id = format!("property:{key}={v}");
                    items.push(("Move up".to_owned(), MenuAct::MoveChild(id.clone(), -1)));
                    items.push(("Move down".to_owned(), MenuAct::MoveChild(id, 1)));
                    items.push((
                        "Rename value…".to_owned(),
                        MenuAct::RenameValue(key.clone(), v.clone()),
                    ));
                }
            }
        }
        NavTarget::Folder(f) => {
            // `buildFolderMenu`: make things in it, then change it.
            let f = f.clone();
            items.push((
                "New note".to_owned(),
                MenuAct::New(f.clone(), NewKind::Note),
            ));
            items.push((
                "New note from template…".to_owned(),
                MenuAct::FromTemplate(f.clone()),
            ));
            for c in &s.templates().commands {
                let name = if c.name.trim().is_empty() {
                    "Unnamed"
                } else {
                    c.name.as_str()
                };
                items.push((format!("New: {name}"), MenuAct::Command(c.id.clone())));
            }
            items.push(("New folder…".to_owned(), MenuAct::NewFolder(f.clone())));
            items.push((
                "New canvas".to_owned(),
                MenuAct::New(f.clone(), NewKind::Canvas),
            ));
            items.push((
                "New base".to_owned(),
                MenuAct::New(f.clone(), NewKind::Base),
            ));
            if n > 0 {
                items.push((
                    format!("Move {n} selected here"),
                    MenuAct::Drop(paths.clone(), f.clone(), false),
                ));
                items.push((
                    format!("Copy {n} selected here"),
                    MenuAct::Drop(paths, f.clone(), true),
                ));
            }
            items.push(("Open all notes".to_owned(), MenuAct::OpenAll(f.clone())));
            let own = s
                .templates()
                .folders
                .iter()
                .any(|t| t.folder.trim_matches('/') == f);
            items.push((
                if own {
                    "Change folder template…"
                } else {
                    "Set folder template…"
                }
                .to_owned(),
                MenuAct::FolderTemplate(f.clone()),
            ));
            // `buildFolderMenu`'s folder note rows.
            if !f.is_empty() && s.settings().folder_notes {
                if s.folder_note(&f).is_some() {
                    items.push((
                        "Detach folder note".to_owned(),
                        MenuAct::DetachFolderNote(f.clone()),
                    ));
                    if let Some(n) = s.folder_note(&f) {
                        items.push((
                            "Delete folder note".to_owned(),
                            MenuAct::Trash(vec![n.to_owned()]),
                        ));
                    }
                } else {
                    items.push((
                        "Create folder note".to_owned(),
                        MenuAct::CreateFolderNote(f.clone()),
                    ));
                }
            }
            if !f.is_empty() {
                items.push(("Rename…".to_owned(), MenuAct::RenameFolder(f.clone())));
                items.push((
                    "Duplicate folder".to_owned(),
                    MenuAct::DuplicateFolder(f.clone()),
                ));
                items.push(("Move folder to…".to_owned(), MenuAct::MoveFolder(f.clone())));
                items.push(("Delete folder".to_owned(), MenuAct::Trash(vec![f])));
            }
        }
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
        // An icon, a colour, a background (`buildFolderMenu`'s style
        // items, `styleMenuBuilder`).
        let at = paint_target(&menu.target);
        let has_icon = s.own_icon(&at).is_some();
        let has_colour = s.colour(&at).is_some();
        let has_bg = s.background(&at).is_some();
        items.push((
            "Set icon…".to_owned(),
            MenuAct::Paint(Paint::Icon, at.clone(), true),
        ));
        if has_icon {
            items.push((
                "Remove icon".to_owned(),
                MenuAct::Paint(Paint::Icon, at.clone(), false),
            ));
        }
        items.push((
            "Set colour…".to_owned(),
            MenuAct::Paint(Paint::Colour, at.clone(), true),
        ));
        if has_colour {
            items.push((
                "Remove colour".to_owned(),
                MenuAct::Paint(Paint::Colour, at.clone(), false),
            ));
        }
        items.push((
            "Set background…".to_owned(),
            MenuAct::Paint(Paint::Background, at.clone(), true),
        ));
        if has_bg {
            items.push((
                "Remove background".to_owned(),
                MenuAct::Paint(Paint::Background, at, false),
            ));
        }
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
    // Extensions' rows (NN's MenusAPI `registerFolderMenu`,
    // `registerTagMenu`, `registerPropertyMenu`): their `[[context]]` on
    // `navigator:folder`, `navigator:tag`, `navigator:property`.
    if let Some(extra) = try_use_context::<MenuExtra>() {
        let (kind, id) = menu_extra_target(&menu.target);
        for (label, command) in (extra.rows)(kind, &id) {
            items.push((
                label,
                MenuAct::Extension(command, kind.to_owned(), id.clone()),
            ));
        }
    }
    let top = menu.top;
    // Kept inside the pane, scrolled when longer (a folder's menu).
    let room = (nav
        .state
        .peek()
        .scroll(crate::layout::Pane::Navigation)
        .view
        - top
        - 8.0)
        .max(120.0);
    let mut open = nav.menu;
    rsx! {
        // A press anywhere else closes it.
        div { class: "nv-menu-backdrop", style: "position:absolute; inset:0; z-index:9;",
            onclick: move |_| open.set(None),
            oncontextmenu: move |e: MouseEvent| { e.prevent_default(); open.set(None); },
        }
        div { class: "nv-menu nv-row-menu", role: "menu",
            style: "position:absolute; left:12px; right:12px; top:{top}px; max-height:{room}px; z-index:10; padding:4px; border-radius:8px; \
                    background:var(--nv-background); border:1px solid var(--nv-divider); display:flex; flex-direction:column; overflow-y:auto;",
            for (label, act) in items {
                button { class: "nv-menu-row", role: "menuitem", "data-menu-item": "{label}",
                    style: "text-align:left; height:26px; padding:0 8px; border:none; border-radius:5px; background:transparent; color:var(--nv-text); cursor:pointer; font-size:12px;",
                    onclick: move |_| {
                        open.set(None);
                        if let MenuAct::Extension(command, kind, id) = &act {
                            if let Some(extra) = try_consume_context::<MenuExtra>() {
                                (extra.run)(command, kind, id);
                            }
                            return;
                        }
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
    RenameKey(String),
    DeleteKey(String),
    CreateFolderNote(String),
    DetachFolderNote(String),
    /// An extension's row: its command, run on the target and its id.
    Extension(String, String, String),
    /// A nested tag (`tag:<t>`) or a value (`property:<k>=<v>`) moved.
    MoveChild(String, isize),
    ResetChildOrder(String),
    RenameValue(String, String),
    /// A shortcut added (`true`) or taken out.
    Shortcut(Shortcut, bool),
    Hide(HideTarget),
    /// A new file of a kind in a folder.
    New(String, NewKind),
    FromTemplate(String),
    /// A "create note" command, by its id.
    Command(String),
    NewFolder(String),
    /// Files moved (or copied) into a folder.
    Drop(Vec<String>, String, bool),
    OpenAll(String),
    FolderTemplate(String),
    RenameFolder(String),
    DuplicateFolder(String),
    MoveFolder(String),
    Trash(Vec<String>),
    /// Painted (asked for a value), or (`false`) the paint taken off.
    Paint(Paint, String, bool),
    Nothing,
}

/// A row's target as icons and colours key it.
fn paint_target(t: &NavTarget) -> String {
    match t {
        NavTarget::Folder(f) => format!("folder:{f}"),
        NavTarget::Tag(t) => format!("tag:{t}"),
        NavTarget::Property { key, value: None } => format!("property:{key}"),
        NavTarget::Property {
            key,
            value: Some(v),
        } => format!("property:{key}={v}"),
    }
}

/// What a paint prompt asks.
pub(super) const fn paint_title(what: Paint) -> &'static str {
    match what {
        Paint::Icon => "Icon: a Lucide name (folder), an emoji, ph:house, or a vault .svg",
        Paint::Colour => "Colour (CSS: #e5484d, tomato, rgb(…))",
        Paint::Background => "Background (CSS colour)",
    }
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
        MenuAct::CreateFolderNote(folder) => {
            match nav.state.peek().folder_note_request(&folder) {
                Ok(req) => nav.new_note(req, on_open),
                Err(e) => nav.say(e),
            }
            return;
        }
        MenuAct::DetachFolderNote(folder) => nav.update(|s| s.detach_folder_note(&folder, None)),
        MenuAct::MoveChild(id, by) => {
            nav.update(|s| s.move_nested(&id, by));
            return;
        }
        MenuAct::ResetChildOrder(id) => {
            nav.update(|s| s.reset_child_order(&id));
            return;
        }
        MenuAct::RenameKey(key) => {
            let name = nav
                .state
                .peek()
                .property_tree()
                .key(&key)
                .map_or_else(|| key.clone(), |k| k.name.clone());
            let mut prompt = nav.prompt;
            prompt.set(Some(Prompt {
                title: format!("Rename property {name}"),
                value: name,
                kind: PromptKind::RenameKey(key),
            }));
            return;
        }
        MenuAct::RenameValue(key, value) => {
            let shown = nav
                .state
                .peek()
                .property_tree()
                .value(&key, &value)
                .map_or_else(|| value.clone(), |n| n.written.clone());
            let mut prompt = nav.prompt;
            prompt.set(Some(Prompt {
                title: format!("Rename {shown}"),
                value: shown,
                kind: PromptKind::RenameValue { key, value },
            }));
            return;
        }
        MenuAct::DeleteKey(key) => match nav.state.peek().delete_property_key(&key) {
            Ok((out, n)) => {
                nav.ask(
                    format!(
                        "Delete {key} from {n} note{}?",
                        if n == 1 { "" } else { "s" }
                    ),
                    "Delete",
                    AskThen::Act(out),
                );
                return;
            }
            Err(e) => Err(e),
        },
        MenuAct::Delete(t) => match nav.state.peek().delete_tag(&t) {
            Ok(out) => {
                nav.ask(
                    format!("Delete #{t} and the tags below it from every note?"),
                    "Delete",
                    AskThen::Act(out),
                );
                return;
            }
            Err(e) => Err(e),
        },
        MenuAct::New(folder, kind) => {
            nav.new_note(
                NewNote {
                    folder,
                    kind,
                    ..NewNote::default()
                },
                on_open,
            );
            return;
        }
        MenuAct::FromTemplate(folder) => {
            // The picker (`TemplateFileModal`): the template folder's notes
            // under the field, narrowed as one types.
            ask_for(
                nav,
                "New note from a template",
                "",
                PromptKind::TemplateFor(folder),
            );
            return;
        }
        MenuAct::Command(id) => {
            nav.run_template_command(&id, on_open);
            return;
        }
        MenuAct::NewFolder(parent) => {
            ask_for(nav, "New folder", "", PromptKind::NewFolder(parent));
            return;
        }
        MenuAct::Drop(paths, folder, copy) => {
            nav.state
                .peek()
                .drop_on(&Drag::Notes(paths), &NavTarget::Folder(folder), copy)
        }
        MenuAct::OpenAll(folder) => {
            let out = nav.update(|s| s.click_folder(&folder));
            nav.act(out, on_open);
            match nav.state.peek().open_all() {
                Ok(paths) => nav.open_many(paths, on_open),
                Err(e) => nav.say(e),
            }
            return;
        }
        MenuAct::FolderTemplate(folder) => {
            let now = nav
                .state
                .peek()
                .templates()
                .folders
                .iter()
                .find(|t| t.folder.trim_matches('/') == folder)
                .map(|t| t.template.clone())
                .unwrap_or_default();
            ask_for(
                nav,
                "Folder template (a vault path; empty, none)",
                &now,
                PromptKind::FolderTemplate(folder),
            );
            return;
        }
        MenuAct::RenameFolder(folder) => {
            let mut renaming = nav.renaming;
            renaming.set(Some(Renaming::Folder(folder)));
            return;
        }
        MenuAct::DuplicateFolder(folder) => nav.state.peek().duplicate_folder(&folder),
        MenuAct::MoveFolder(folder) => {
            ask_for(
                nav,
                "Move to folder (a vault path; empty, the root)",
                "",
                PromptKind::MoveFolderTo(folder),
            );
            return;
        }
        MenuAct::Trash(paths) => {
            nav.trash(&paths, on_open);
            return;
        }
        MenuAct::Paint(what, target, true) => {
            let now = {
                let s = nav.state.peek();
                match what {
                    Paint::Icon => s.own_icon(&target).map(|i| i.id()),
                    Paint::Colour => s.colour(&target).map(str::to_owned),
                    Paint::Background => s.background(&target).map(str::to_owned),
                }
                .unwrap_or_default()
            };
            ask_for(
                nav,
                paint_title(what),
                &now,
                PromptKind::Paint { what, target },
            );
            return;
        }
        MenuAct::Paint(what, target, false) => match nav.update(|s| s.paint(what, &target, None)) {
            Ok(Some(out)) => Ok(out),
            Ok(None) => return,
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
        MenuAct::Nothing | MenuAct::Extension(..) => return,
    };
    match out {
        Ok(out) => nav.act(Some(out), on_open),
        Err(e) => nav.say(e),
    }
}

// ── Asking ─────────────────────────────────────────────────────────────

/// Asks for a name: `title` over a field holding `value`.
pub(super) fn ask_for(nav: Navigator, title: &str, value: &str, kind: PromptKind) {
    let mut prompt = nav.prompt;
    prompt.set(Some(Prompt {
        title: title.to_owned(),
        value: value.to_owned(),
        kind,
    }));
}

/// What a name typed into `kind`'s bar is warned of (a note's or a
/// folder's name): `fileNameUtils`' checks.
fn name_warning(kind: &PromptKind, text: &str) -> Option<String> {
    match kind {
        PromptKind::NewFolder(_) | PromptKind::NewNamed { .. } | PromptKind::Merge(_) => {
            let c = check_name(text);
            c.error.or(c.warning)
        }
        _ => None,
    }
}

/// What a prompt's text asks of the navigator, when it is given.
fn submit_prompt(
    nav: Navigator,
    on_open: EventHandler<Open>,
    kind: &PromptKind,
    text: &str,
) -> Result<Option<Outcome>, String> {
    let s = nav.state.peek();
    let out = match kind {
        PromptKind::RenameTag(from) => s.rename_tag(from, text),
        PromptKind::RenameKey(key) => {
            let (out, both) = s.rename_property_key(key, text)?;
            if both > 0 {
                drop(s);
                // `confirmRenameConflicts`: theirs is replaced.
                nav.ask(
                    format!(
                        "{both} note{} have {} already: their value is replaced by {key}'s. Rename?",
                        if both == 1 { "" } else { "s" },
                        text.trim()
                    ),
                    "Rename",
                    AskThen::Act(out),
                );
                return Ok(None);
            }
            Ok(out)
        }
        PromptKind::RenameValue { key, value } => s.rename_property_value(key, value, text),
        PromptKind::AddTag(paths) => s.add_tag(paths, text),
        PromptKind::SetValue { key, paths } => s.set_property(paths, key, Some(text)),
        PromptKind::NewFolder(parent) => s.new_folder(parent, text),
        PromptKind::MoveTo(paths) => s.move_files(paths, text.trim_matches('/')),
        PromptKind::MoveFolderTo(folder) => s.move_folder(folder, text.trim_matches('/')),
        PromptKind::Merge(paths) => s.merge(paths, text, None, MergeSeparator::BlankLine, false),
        PromptKind::Paint { what, target } => {
            drop(s);
            let value = Some(text).filter(|t| !t.trim().is_empty());
            return nav.update(|s| s.paint(*what, target, value));
        }
        PromptKind::FolderTemplate(folder) => {
            drop(s);
            let (folder, template) = (folder.clone(), text.to_owned());
            nav.update(|s| s.edit_templates(|t| t.set_folder(&folder, &template, true)));
            nav.say(if text.trim().is_empty() {
                "The folder has no template now"
            } else {
                "New notes here and below take the template"
            });
            return Ok(None);
        }
        PromptKind::NewNamed { folder, kind } => {
            drop(s);
            nav.new_note(
                NewNote {
                    folder: folder.clone(),
                    kind: *kind,
                    name: Some(text.to_owned()),
                    ..NewNote::default()
                },
                on_open,
            );
            return Ok(None);
        }
        PromptKind::TemplateFor(folder) => {
            drop(s);
            nav.new_note(
                NewNote {
                    folder: folder.clone(),
                    template: Some(text.trim().to_owned()).filter(|t| !t.is_empty()),
                    ..NewNote::default()
                },
                on_open,
            );
            return Ok(None);
        }
        PromptKind::Template { req, label } => {
            drop(s);
            let mut req = (**req).clone();
            req.prompts.insert(label.clone(), text.to_owned());
            nav.new_note(req, on_open);
            return Ok(None);
        }
    };
    out.map(Some)
}

/// A bar over the navigator's foot asking for a name: what it would break
/// said under it as it is typed.
#[component]
pub(super) fn PromptBar(nav: Navigator, on_open: EventHandler<Open>, prompt: Prompt) -> Element {
    let mut value = use_signal(|| prompt.value.clone());
    let mut open = nav.prompt;
    let kind = prompt.kind.clone();
    let warning = name_warning(&kind, &value.read());
    let submit = move || {
        let text = value.peek().trim().to_owned();
        open.set(None);
        match submit_prompt(nav, on_open, &kind, &text) {
            Ok(Some(out)) => nav.act(Some(out), on_open),
            Ok(None) => {}
            Err(e) => {
                nav.say(e);
            }
        }
    };
    let mut on_enter = submit.clone();
    let mut on_ok = submit.clone();
    let on_pick = submit;
    let choices: Vec<String> = match &prompt.kind {
        PromptKind::TemplateFor(_) => {
            let want = value.read().trim().to_lowercase();
            nav.state
                .peek()
                .template_notes()
                .into_iter()
                .filter(|t| want.is_empty() || t.to_lowercase().contains(&want))
                .take(8)
                .collect()
        }
        _ => Vec::new(),
    };
    rsx! {
        div { class: "nv-confirm nv-prompt", role: "dialog", "aria-label": "{prompt.title}",
            style: "position:absolute; left:8px; right:8px; bottom:8px; z-index:20; padding:10px; border-radius:10px; \
                    background:var(--nv-background); border:1px solid var(--nv-divider); display:flex; flex-direction:column; gap:8px;",
            div { style: "font-weight:600;", "{prompt.title}" }
            input { class: "nv-prompt-input", "data-testid": "nv-prompt-input", value: "{value}",
                autofocus: true,
                style: "height:26px; padding:0 8px; border-radius:6px; border:1px solid var(--nv-divider); background:transparent; color:var(--nv-text);",
                oninput: move |e| value.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == Key::Named(NamedKey::Enter) {
                        on_enter();
                    } else if e.key() == Key::Named(NamedKey::Escape) {
                        open.set(None);
                    }
                },
            }
            if !choices.is_empty() {
                div { class: "nv-prompt-choices", role: "listbox", style: "display:flex; flex-direction:column; gap:2px; max-height:200px; overflow-y:auto;",
                    for c in choices {
                        {
                            let pick = c.clone();
                            let mut on_pick = on_pick.clone();
                            let shown = c.trim_end_matches(".md").to_owned();
                            rsx! {
                                button { key: "{c}", class: "nv-prompt-choice", role: "option",
                                    style: "text-align:left; height:24px; padding:0 8px; border:none; border-radius:6px; background:transparent; color:var(--nv-text); cursor:pointer; overflow:hidden; text-overflow:ellipsis; white-space:nowrap;",
                                    onclick: move |_| { value.set(pick.clone()); on_pick(); },
                                    "{shown}"
                                }
                            }
                        }
                    }
                }
            }
            if let Some(w) = warning {
                div { class: "nv-name-warning", "data-testid": "nv-name-warning", role: "status",
                    style: "font-size:11px; color:var(--nv-muted); overflow-wrap:anywhere;", "{w}" }
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

/// A bar asking a yes before a write that changes many notes, puts things
/// in the trash, or opens many notes.
#[component]
pub(super) fn AskBar(nav: Navigator, on_open: EventHandler<Open>, ask: Ask) -> Element {
    let mut open = nav.ask;
    let Ask { text, then, ok } = ask;
    rsx! {
        div { class: "nv-confirm nv-ask", role: "alertdialog", "aria-label": "{text}",
            style: "position:absolute; left:8px; right:8px; bottom:8px; z-index:20; padding:10px; border-radius:10px; \
                    background:var(--nv-background); border:1px solid var(--nv-divider); display:flex; flex-direction:column; gap:8px;",
            div { style: "overflow-wrap:anywhere;", "{text}" }
            div { style: "display:flex; justify-content:flex-end; gap:6px;",
                button { class: "nv-confirm-cancel", "data-testid": "nv-ask-cancel",
                    style: "height:26px; padding:0 10px; border-radius:6px; border:1px solid var(--nv-divider); background:transparent; color:var(--nv-text); cursor:pointer;",
                    onclick: move |_| open.set(None),
                    "Cancel"
                }
                button { class: "nv-confirm-create", "data-testid": "nv-ask-ok",
                    style: "height:26px; padding:0 10px; border-radius:6px; border:none; background:var(--nv-open); color:var(--nv-background); cursor:pointer; font-weight:600;",
                    onclick: move |_| {
                        open.set(None);
                        nav.then(then.clone(), on_open);
                    },
                    "{ok}"
                }
            }
        }
    }
}

/// A note's or a folder's name being edited in place (`InlineRenameInput`):
/// Return renames, Escape or leaving it lets go; what the name would break
/// is said under it.
#[component]
pub(super) fn InlineRename(
    nav: Navigator,
    on_open: EventHandler<Open>,
    what: Renaming,
    value: String,
) -> Element {
    let mut text = use_signal(|| value.clone());
    let mut renaming = nav.renaming;
    let check = check_name(&text.read());
    let note = check.error.clone().or_else(|| check.warning.clone());
    let w1 = what;
    let done = move || {
        let name = text.peek().trim().to_owned();
        renaming.set(None);
        if name == value.trim() {
            return;
        }
        let out = match &w1 {
            Renaming::File(p) => nav.state.peek().rename_file(p, &name),
            Renaming::Folder(f) => nav.state.peek().rename_folder(f, &name),
        };
        match out {
            Ok(out) => nav.act(Some(out), on_open),
            Err(e) => nav.say(e),
        }
    };
    let mut on_enter = done;
    rsx! {
        span { class: "nv-rename", style: "flex:1 1 auto; min-width:0; display:flex; flex-direction:column; position:relative;",
            input { class: "nv-rename-input", "data-testid": "nv-rename-input", value: "{text}",
                autofocus: true,
                "aria-label": "Name",
                style: "height:22px; padding:0 4px; border-radius:4px; border:1px solid var(--nv-open); background:var(--nv-background); color:var(--nv-text); font-size:13px;",
                onclick: move |e| e.stop_propagation(),
                oninput: move |e| text.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    match e.key() {
                        Key::Named(NamedKey::Enter) => on_enter(),
                        Key::Named(NamedKey::Escape) => renaming.set(None),
                        _ => {}
                    }
                },
            }
            if let Some(n) = note {
                span { class: "nv-name-warning", "data-testid": "nv-name-warning", role: "status",
                    style: "position:absolute; top:24px; left:0; right:0; z-index:5; padding:2px 6px; border-radius:4px; \
                            background:var(--nv-background); border:1px solid var(--nv-divider); font-size:11px; color:var(--nv-muted); white-space:normal;",
                    "{n}"
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
                // The search box's keys (`search:close`, `search:focus-list`,
                // `search:focus-navigation`).
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    let ev = super::key_event(&e);
                    let action = nav.state.peek().keymap().action(crate::KeyScope::Search, &ev, super::MAC);
                    match action {
                        Some(crate::KeyAction::SearchClose) => nav.update(crate::NavigatorState::toggle_search),
                        Some(a) => {
                            e.prevent_default();
                            nav.update(|s| s.run_key(a));
                        }
                        None => {}
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
    /// Under a drag that it takes (the shortcuts', to add at the top).
    drop_here: bool,
) -> Element {
    let toggle = id.clone();
    let class = if drop_here {
        "nv-row nv-section nv-drop-target"
    } else {
        "nv-row nv-section"
    };
    rsx! {
        div { class, role: "treeitem", "data-section": "{id}",
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
    /// Under a drag that it takes.
    drop_here: bool,
) -> Element {
    let badge = (at < 9).then(|| at.saturating_add(1).to_string());
    let class = if drop_here {
        "nv-row nv-shortcut nv-drop-target"
    } else {
        "nv-row nv-shortcut"
    };
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
        div { class, role: "treeitem", "data-shortcut": "{at}", "data-kind": "{kind.id()}",
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; display:flex; align-items:center; gap:6px; \
                    padding:0 8px 0 10px; border-radius:6px; white-space:nowrap; color:{colour};",
            onclick: move |_| {
                if nav.dragged.cloned() {
                    return;
                }
                match nav.update(|s| s.open_shortcut_entry(&open)) {
                    Ok(out) => nav.act(out, on_open),
                    Err(e) => nav.say(e),
                }
            },
            // Dragged to another place among them (`SHORTCUT_POINTER_CONSTRAINT`).
            onpointerdown: move |e: PointerEvent| start_drag(nav, Drag::Shortcut(at), &e),
            onpointermove: move |e: PointerEvent| drag_move(nav, &e),
            onpointerup: move |_| drag_end(nav, on_open),
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
    // Its own icon or its type's (a project's note: the project's glyph).
    let (icon, colour) = {
        let s = nav.state.read();
        (
            s.file_icon(&path),
            s.colour(&format!("file:{path}")).map(str::to_owned),
        )
    };
    rsx! {
        div { class, role: "treeitem", "data-recent": "{path}",
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; display:flex; align-items:center; gap:6px; \
                    padding:0 8px 0 10px; border-radius:6px; white-space:nowrap;",
            onclick: move |_| {
                let out = nav.state.peek().open_recent(&p);
                nav.act(out, on_open);
            },
            span { style: "flex:0 0 auto; display:flex; color:var(--nv-faint);",
                if let Some(icon) = icon {
                    super::icon::IconView { nav, icon, size: 15, colour }
                } else {
                    FileText { size: 15 }
                }
            }
            span { class: "nv-name", style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis;", "{name}" }
        }
    }
}

/// The modifiers as the model takes them, for a click on a pill.
pub(super) fn click_mods(e: &MouseEvent) -> Mods {
    super::panes::mods(e.modifiers())
}

/// The name's `flex`: it takes the room, or (a count leader drawn) only
/// its own.
pub(super) fn name_flex(nav: Navigator, counted: bool) -> &'static str {
    let leader = nav.state.peek().settings().count_leader;
    if counted && leader != crate::settings::CountLeader::None {
        "0 1 auto"
    } else {
        "1 1 auto"
    }
}

/// Between a name and its count (`navCountLeaderStyle`): dots, dashes or
/// a line.
#[component]
pub(super) fn Leader(nav: Navigator) -> Element {
    let style = match nav.state.peek().settings().count_leader {
        crate::settings::CountLeader::None => return rsx! {},
        crate::settings::CountLeader::Dots => "dotted",
        crate::settings::CountLeader::Dashes => "dashed",
        crate::settings::CountLeader::Line => "solid",
    };
    rsx! {
        span { class: "nv-count-leader",
            style: "flex:1 1 auto; min-width:8px; align-self:center; margin:0 4px; height:0; border-bottom:1px {style} var(--nv-faint); opacity:0.6;" }
    }
}

/// Lines down the levels (`showIndentGuides`), under a row's chevron.
#[component]
pub(super) fn Guides(nav: Navigator, level: u16, indent: f64) -> Element {
    if !nav.state.peek().settings().indent_guides || level == 0 {
        return rsx! {};
    }
    rsx! {
        for l in 0..level {
            span { key: "{l}", class: "nv-indent-guide",
                style: "position:absolute; top:0; bottom:0; width:1px; left:{f64::from(l).mul_add(indent, 13.0)}px; background:var(--nv-divider); pointer-events:none;" }
        }
    }
}

/// A folder's or note's tooltip (`showTooltips`): its path, tags and
/// words, as the settings say; `None` with them off.
pub(super) fn tooltip(nav: Navigator, path: &str) -> Option<String> {
    let s = nav.state.peek();
    let st = s.settings();
    if !st.tooltips {
        return None;
    }
    let mut out = vec![
        path.rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".md")
            .to_owned(),
    ];
    if st.tooltip_path && path.contains('/') {
        out.push(path.to_owned());
    }
    if let Some(f) = s.index().file(path) {
        if st.tooltip_tags && !f.tags.is_empty() {
            out.push(
                f.tags
                    .iter()
                    .map(|t| format!("#{t}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
        }
        if st.tooltip_words
            && let Some(w) = f.words.filter(|w| *w > 0)
        {
            out.push(format!("{w} words"));
        }
    }
    Some(out.join("\n"))
}

/// Rows others add to the navigator's menus (Notebook Navigator's
/// `MenusAPI`).
///
/// Given by the app over its extensions' `[[context]]` on
/// `navigator:folder`, `navigator:tag` and `navigator:property`; each is
/// a label and the command it runs, told the target and its id (a
/// folder's path, a tag, `key` or `key=value`).
#[derive(Clone)]
pub struct MenuExtra {
    pub rows: std::rc::Rc<dyn Fn(&str, &str) -> Vec<(String, String)>>,
    pub run: std::rc::Rc<dyn Fn(&str, &str, &str)>,
}

/// The rows an app loads as a deck's context, by path, in context.
///
/// A project's folder, a setlist's song: a ⌘-click or a middle-click on one
/// opens it with [`crate::OpenIn::NewDeck`] (a ⌘-click on another note
/// still selects it).
#[derive(Clone)]
pub struct DeckRows(pub std::rc::Rc<dyn Fn(&str) -> bool>);

/// The app's [`DeckRows`], read where a row is drawn (an event handler
/// has no scope to look it up from).
pub(super) fn use_deck_rows() -> Option<DeckRows> {
    try_use_context::<DeckRows>()
}

/// Whether `path` is a row the app loads as a deck.
pub(super) fn deck_row(rows: Option<&DeckRows>, path: &str) -> bool {
    rows.is_some_and(|d| (d.0)(path))
}

/// Opens `path` onto a new deck.
pub(super) fn open_deck(path: &str, on_open: EventHandler<Open>) {
    on_open.call(Open {
        path: path.to_owned(),
        how: crate::OpenIn::NewDeck,
        browsing: false,
        cursor: None,
    });
}

/// A middle-click on row `path`: onto a new deck when it is one, else a
/// new tab. True when it was a middle-click.
pub(super) fn middle_click(
    e: &PointerEvent,
    rows: Option<&DeckRows>,
    path: &str,
    on_open: EventHandler<Open>,
) -> bool {
    if e.data().trigger_button() != Some(dioxus::html::input_data::MouseButton::Auxiliary) {
        return false;
    }
    if deck_row(rows, path) {
        open_deck(path, on_open);
    } else if !path.is_empty() {
        on_open.call(Open {
            path: path.to_owned(),
            how: crate::OpenIn::Tab,
            browsing: false,
            cursor: None,
        });
    }
    true
}

/// A menu's target as others' rows name it: `navigator:<kind>` and its id.
#[must_use]
pub fn menu_extra_target(target: &NavTarget) -> (&'static str, String) {
    match target {
        NavTarget::Folder(f) => ("navigator:folder", f.clone()),
        NavTarget::Tag(t) => ("navigator:tag", t.clone()),
        NavTarget::Property { key, value } => (
            "navigator:property",
            value
                .as_ref()
                .map_or_else(|| key.clone(), |v| format!("{key}={v}")),
        ),
    }
}
