//! The two panes: the folder tree (Notebook Navigator's `NavigationPane`,
//! `FolderItem`) and the notes (`ListPane`, `FileItem`), each a header and
//! a virtualized body.

use dioxus::prelude::*;
use lucide_dioxus::{
    ArrowDown, ArrowDownWideNarrow, ArrowLeft, ArrowUp, ArrowUpDown, Check, ChevronDown,
    ChevronRight, ChevronsDownUp, ChevronsUpDown, Columns2, Folder, FolderOpen, ListTree, Vault,
};

use super::virtual_pane::VirtualPane;
use super::{Navigator, Open};
use crate::index::NoteCount;
use crate::layout::Pane;
use crate::list::{self, Clock, ListRowKind};
use crate::settings::{ListMode, SortOption};
use crate::source::Millis;
use crate::state::{Mods, NavigatorState};
use crate::tree::NavRowKind;

/// A pane's header: the tools along its top.
const HEADER: f64 = 36.0;

/// The modifiers of a click, ⌘ on a Mac and Ctrl elsewhere as the command.
fn mods(m: Modifiers) -> Mods {
    Mods {
        shift: m.shift(),
        command: if cfg!(target_os = "macos") {
            m.meta()
        } else {
            m.ctrl()
        },
    }
}

#[component]
pub(super) fn NavPane(nav: Navigator, on_open: EventHandler<Open>) -> Element {
    let s = nav.state.read();
    let any = s.any_expanded();
    let reordering = s.reordering;
    let dual = s.settings().dual_pane;
    drop(s);
    rsx! {
        div { class: "nv-nav", "data-pane": "navigation",
            style: "position:absolute; inset:0; display:flex; flex-direction:column; background:var(--nv-background);",
            div { class: "nv-header",
                style: "flex:0 0 {HEADER}px; display:flex; align-items:center; gap:2px; padding:0 6px; border-bottom:1px solid var(--nv-divider);",
                button { class: "nv-tool", "aria-label": if any { "Collapse all folders" } else { "Expand all folders" },
                    onclick: move |_| nav.update(NavigatorState::collapse_or_expand_all),
                    if any { ChevronsDownUp { size: 16 } } else { ChevronsUpDown { size: 16 } }
                }
                button { class: if reordering { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Reorder the top folders",
                    onclick: move |_| nav.update(|s| s.reordering = !s.reordering),
                    ArrowUpDown { size: 16 }
                }
                div { style: "flex:1;" }
                button { class: if dual { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Two panes",
                    onclick: move |_| nav.settings.update(|s| s.dual_pane = !s.dual_pane),
                    Columns2 { size: 16 }
                }
            }
            div { style: "flex:1 1 0; min-height:0; position:relative;",
                VirtualPane { nav, pane: Pane::Navigation, label: "Folders", NavRows { nav } }
            }
        }
    }
}

#[component]
fn NavRows(nav: Navigator) -> Element {
    let s = nav.state.read();
    let window = s.window(Pane::Navigation);
    let offset = s.scroll(Pane::Navigation).offset;
    let indent = f64::from(s.settings().indent);
    let show_count = s.settings().show_note_count;
    let separate = s.settings().separate_note_counts;
    let selected = s.selection().folder().map(str::to_owned);
    let reordering = s.reordering;
    let root_level = u16::from(s.settings().show_root);
    // Collected, so the state is let go before the rows are drawn.
    #[allow(clippy::needless_collect)]
    let rows: Vec<_> = window
        .filter_map(|i| {
            let row = s.nav_rows().get(i)?;
            let top = s.nav_offsets().start(i) - offset;
            let h = s.nav_offsets().height(i);
            Some((row.clone(), top, h))
        })
        .collect();
    drop(s);
    rsx! {
        for (row, top, h) in rows {
            if let NavRowKind::Folder { path, name, has_children, expanded, count } = row.kind {
                FolderRow {
                    key: "{row.key}",
                    nav,
                    selected: selected.as_deref() == Some(path.as_str()),
                    path: path.clone(),
                    name,
                    level: row.level,
                    has_children,
                    expanded,
                    count: count_label(count, show_count, separate),
                    top,
                    height: h,
                    indent,
                    reorder: reordering && row.level == root_level && !path.is_empty(),
                }
            }
        }
    }
}

/// A folder's count as Notebook Navigator writes it: "3 • 12" (its own,
/// then below), "• 12" with none of its own, nothing at zero.
fn count_label(c: NoteCount, show: bool, separate: bool) -> String {
    if !show {
        return String::new();
    }
    if separate {
        match (c.current, c.descendants) {
            (0, 0) => String::new(),
            (n, 0) => n.to_string(),
            (0, d) => format!("• {d}"),
            (n, d) => format!("{n} • {d}"),
        }
    } else if c.total() == 0 {
        String::new()
    } else {
        c.total().to_string()
    }
}

#[component]
fn FolderRow(
    nav: Navigator,
    selected: bool,
    path: String,
    name: String,
    level: u16,
    has_children: bool,
    expanded: bool,
    count: String,
    top: f64,
    height: f64,
    indent: f64,
    reorder: bool,
) -> Element {
    let pad = f64::from(level).mul_add(indent, 6.0);
    let root = path.is_empty();
    let class = if selected {
        "nv-row nv-selected"
    } else {
        "nv-row"
    };
    let name_weight = if root { 600 } else { 400 };
    let (p1, p2, p3, p4) = (path.clone(), path.clone(), name.clone(), name.clone());
    rsx! {
        div {
            class,
            role: "treeitem",
            "data-folder": "{path}",
            "aria-selected": if selected { "true" } else { "false" },
            "aria-expanded": if has_children { if expanded { "true" } else { "false" } } else { "" },
            "aria-level": "{level}",
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; display:flex; align-items:center; \
                    gap:4px; padding-left:{pad}px; padding-right:8px; border-radius:6px; white-space:nowrap;",
            onclick: move |_| {
                if nav.dragged.cloned() {
                    return;
                }
                nav.update(|s| s.click_folder(&p1));
            },
            // The chevron opens and closes without selecting.
            div {
                class: "nv-chevron",
                "aria-label": if expanded { "Collapse" } else { "Expand" },
                style: "flex:0 0 16px; height:16px; display:flex; align-items:center; justify-content:center; color:var(--nv-faint);",
                onclick: move |e| {
                    e.stop_propagation();
                    if has_children {
                        nav.update(|s| s.toggle(&p2));
                    }
                },
                if has_children {
                    if expanded { ChevronDown { size: 14 } } else { ChevronRight { size: 14 } }
                }
            }
            span { style: "flex:0 0 auto; display:flex; color:var(--nv-faint);",
                if root { Vault { size: 15 } } else if expanded { FolderOpen { size: 15 } } else { Folder { size: 15 } }
            }
            span { class: "nv-name", style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; font-weight:{name_weight};", "{name}" }
            if reorder {
                button { class: "nv-tool", "aria-label": "Move {p3} up", style: "width:20px; height:20px;",
                    onclick: move |e| { e.stop_propagation(); nav.update(|s| s.move_child("", &p3, -1)); },
                    ArrowUp { size: 13 }
                }
                button { class: "nv-tool", "aria-label": "Move {p4} down", style: "width:20px; height:20px;",
                    onclick: move |e| { e.stop_propagation(); nav.update(|s| s.move_child("", &p4, 1)); },
                    ArrowDown { size: 13 }
                }
            } else if !count.is_empty() {
                span { class: "nv-count", style: "flex:0 0 auto; color:var(--nv-faint); font-size:11px;", "{count}" }
            }
        }
    }
}

#[component]
pub(super) fn ListPane(
    nav: Navigator,
    on_open: EventHandler<Open>,
    on_context_menu: Option<EventHandler<(String, MouseEvent)>>,
    single: bool,
) -> Element {
    let s = nav.state.read();
    let folder = s.selection().folder().unwrap_or("").to_owned();
    let title = s.index().folder_name(&folder).to_owned();
    let sort = s.settings().sort;
    let descendants = s.settings().include_descendants;
    let empty = s.list_order().is_empty();
    drop(s);
    let mut menu = use_signal(|| false);
    rsx! {
        div { class: "nv-list", "data-pane": "files",
            style: "position:absolute; inset:0; display:flex; flex-direction:column; background:var(--nv-list-background);",
            div { class: "nv-header",
                style: "flex:0 0 {HEADER}px; display:flex; align-items:center; gap:2px; padding:0 6px; border-bottom:1px solid var(--nv-divider); position:relative; z-index:4;",
                if single {
                    button { class: "nv-tool", "aria-label": "Back to the folders",
                        onclick: move |_| nav.update(|s| s.focus_pane(Pane::Navigation)),
                        ArrowLeft { size: 16 }
                    }
                }
                span { class: "nv-title", style: "flex:1 1 auto; min-width:0; overflow:hidden; text-overflow:ellipsis; white-space:nowrap; font-weight:600; padding:0 4px;", "{title}" }
                button { class: if descendants { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Notes from folders below",
                    onclick: move |_| nav.settings.update(|s| s.include_descendants = !s.include_descendants),
                    ListTree { size: 16 }
                }
                button { class: if menu() { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Sort",
                    onclick: move |_| menu.toggle(),
                    ArrowDownWideNarrow { size: 16 }
                }
                if menu() {
                    div { class: "nv-menu", role: "menu",
                        style: "position:absolute; top:{HEADER}px; right:6px; z-index:10; min-width:220px; padding:4px; border-radius:8px; \
                                background:var(--nv-background); border:1px solid var(--nv-divider);",
                        for option in SortOption::ALL {
                            div { class: "nv-menu-row", role: "menuitemradio",
                                key: "{option.id()}",
                                "data-sort": "{option.id()}",
                                "aria-checked": if option == sort { "true" } else { "false" },
                                style: "display:flex; align-items:center; gap:6px; height:26px; padding:0 8px; border-radius:5px; white-space:nowrap;",
                                onclick: move |_| {
                                    nav.settings.update(|s| s.sort = option);
                                    menu.set(false);
                                },
                                span { style: "flex:0 0 14px; display:flex;", if option == sort { Check { size: 13 } } }
                                "{option.label()}"
                            }
                        }
                    }
                }
            }
            div { style: "flex:1 1 0; min-height:0; position:relative;",
                if empty {
                    div { style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; flex-direction:column; justify-content:center; padding:16px;",
                        p { style: "margin:0; text-align:center; color:var(--nv-faint); font-size:12px;", "No notes here" }
                    }
                } else {
                    VirtualPane { nav, pane: Pane::Files, label: "Notes", ListRows { nav, on_open, on_context_menu } }
                }
            }
        }
    }
}

#[component]
fn ListRows(
    nav: Navigator,
    on_open: EventHandler<Open>,
    on_context_menu: Option<EventHandler<(String, MouseEvent)>>,
) -> Element {
    let s = nav.state.read();
    let window = s.window(Pane::Files);
    let offset = s.scroll(Pane::Files).offset;
    let settings = s.settings().clone();
    let clock = s.clock();
    let active = s.active().map(str::to_owned);
    // Collected, so the state is let go before the rows are drawn.
    #[allow(clippy::needless_collect)]
    let rows: Vec<_> = window
        .filter_map(|i| {
            let row = s.list_rows().get(i)?;
            let top = s.list_offsets().start(i) - offset;
            let h = s.list_offsets().height(i);
            match &row.kind {
                ListRowKind::Header { label } => Some(Item::Header {
                    key: row.key.clone(),
                    label: label.clone(),
                    top,
                    h,
                }),
                ListRowKind::File { path, parent, .. } => {
                    let f = s.file(path)?;
                    let at = list::date_of(f, settings.sort);
                    Some(Item::File(FileItem {
                        path: path.clone(),
                        name: f.title(settings.title_from_frontmatter).to_owned(),
                        preview: f.preview.clone().unwrap_or_default(),
                        date: date_label(at, clock),
                        parent: parent.clone(),
                        selected: s.selection().is_selected(path),
                        open: active.as_deref() == Some(path.as_str()),
                        top,
                        h,
                    }))
                }
                _ => None,
            }
        })
        .collect();
    drop(s);
    let compact = settings.mode == ListMode::Compact;
    rsx! {
        for item in rows {
            match item {
                Item::Header { key, label, top, h } => rsx! {
                    div { key: "{key}", class: "nv-group", role: "heading",
                        style: "position:absolute; left:0; right:0; top:{top}px; height:{h}px; display:flex; align-items:flex-end; \
                                padding:0 14px 5px; color:var(--nv-heading); font-size:12px; font-weight:600;",
                        "{label}"
                    }
                },
                Item::File(f) => rsx! {
                    FileRow {
                        key: "{f.path}",
                        nav,
                        on_open,
                        on_context_menu,
                        item: f,
                        compact,
                        preview_rows: if settings.show_preview { settings.preview_rows.clamp(1, 5) } else { 0 },
                        title_rows: settings.title_rows.clamp(1, 3),
                        show_date: settings.show_date,
                        show_parent: settings.show_parent_folder,
                    }
                },
            }
        }
    }
}

enum Item {
    Header {
        key: String,
        label: String,
        top: f64,
        h: f64,
    },
    File(FileItem),
}

#[derive(Clone, PartialEq)]
struct FileItem {
    path: String,
    name: String,
    preview: String,
    date: String,
    parent: Option<String>,
    selected: bool,
    open: bool,
    top: f64,
    h: f64,
}

#[component]
fn FileRow(
    nav: Navigator,
    on_open: EventHandler<Open>,
    on_context_menu: Option<EventHandler<(String, MouseEvent)>>,
    item: FileItem,
    compact: bool,
    preview_rows: u32,
    title_rows: u32,
    show_date: bool,
    show_parent: bool,
) -> Element {
    let FileItem {
        path,
        name,
        preview,
        date,
        parent,
        selected,
        open,
        top,
        h,
    } = item;
    let class = if selected {
        "nv-row nv-file nv-selected"
    } else {
        "nv-row nv-file"
    };
    let (p1, p2) = (path.clone(), path.clone());
    let title_h = f64::from(title_rows) * 20.0;
    let preview_h = f64::from(preview_rows) * 18.0;
    let weight = if open { 700 } else { 600 };
    let (justify, padding) = if compact {
        ("center", "0 10px")
    } else {
        ("flex-start", "8px 10px")
    };
    let wrap = if title_rows == 1 { "nowrap" } else { "normal" };
    rsx! {
        div {
            class,
            role: "option",
            "data-file": "{path}",
            "aria-selected": if selected { "true" } else { "false" },
            "aria-current": if open { "page" } else { "false" },
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{h}px; border-radius:6px; overflow:hidden; \
                    display:flex; flex-direction:column; justify-content:{justify}; \
                    padding:{padding}; box-sizing:border-box;",
            onclick: move |e: MouseEvent| {
                if nav.dragged.cloned() {
                    return;
                }
                let out = nav.state.clone().write().click_file(&p1, mods(e.modifiers()));
                nav.outcome(out, on_open, None);
            },
            oncontextmenu: move |e: MouseEvent| {
                if let Some(menu) = on_context_menu {
                    menu.call((p2.clone(), e));
                }
            },
            div { class: "nv-file-name",
                style: "flex:0 0 auto; max-height:{title_h}px; line-height:20px; overflow:hidden; text-overflow:ellipsis; \
                        white-space:{wrap}; font-weight:{weight}; color:var(--nv-text);",
                "{name}"
            }
            if !compact {
                if preview_rows > 0 && !preview.trim().is_empty() {
                    div { class: "nv-file-preview",
                        style: "flex:0 0 auto; max-height:{preview_h}px; line-height:18px; overflow:hidden; color:var(--nv-muted); font-size:12px;",
                        "{preview}"
                    }
                }
                if show_date || (show_parent && parent.is_some()) {
                    div { class: "nv-file-second-line",
                        style: "flex:0 0 auto; height:19px; display:flex; align-items:center; gap:8px; white-space:nowrap; overflow:hidden; color:var(--nv-faint); font-size:12px;",
                        if show_date { span { class: "nv-file-date", "{date}" } }
                        if show_parent && let Some(p) = parent {
                            span { class: "nv-file-parent", style: "display:flex; align-items:center; gap:3px; overflow:hidden; text-overflow:ellipsis;",
                                Folder { size: 11 }
                                "{p}"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A note's date as Notebook Navigator shows it: the time for today and
/// yesterday, the weekday within a week, else the date (`h:mm a`, `dddd`,
/// `MMM D, YYYY`).
fn date_label(at: Millis, clock: Clock) -> String {
    use chrono::{FixedOffset, TimeZone as _};
    let offset = i32::try_from(clock.offset / 1000).unwrap_or(0);
    let Some(tz) = FixedOffset::east_opt(offset) else {
        return String::new();
    };
    let Some(t) = tz.timestamp_millis_opt(at).single() else {
        return String::new();
    };
    let days = clock.day(clock.now).saturating_sub(clock.day(at));
    match days {
        0 | 1 => t.format("%-I:%M %p").to_string(),
        2..=6 => t.format("%A").to_string(),
        _ => t.format("%b %-d, %Y").to_string(),
    }
}
