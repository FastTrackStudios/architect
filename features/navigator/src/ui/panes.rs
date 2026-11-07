//! The two panes: the folder tree (Notebook Navigator's `NavigationPane`,
//! `FolderItem`) and the notes (`ListPane`, `FileItem`), each a header and
//! a virtualized body.

use dioxus::prelude::*;
use lucide_dioxus::{
    ArrowDown, ArrowDownWideNarrow, ArrowLeft, ArrowUp, ArrowUpDown, CalendarDays, Check,
    ChevronDown, ChevronRight, ChevronsDownUp, ChevronsUpDown, Columns2, Eye, EyeOff, FileText,
    Folder, FolderOpen, ListTree, Pin, Rows3, Search, SquarePen, Tag, UserRound, Vault,
};

use super::calendar::Calendar;
use super::tree_rows::{self, ItemRow, RowIcon};
use super::virtual_pane::VirtualPane;
use super::{Navigator, Open, Renaming};
use crate::index::NoteCount;
use crate::layout::Pane;
use crate::list::{self, Clock, Lines, ListRowKind, Thumb};
use crate::selection::NavTarget;
use crate::settings::{CountPlacement, Grouping, ListMode, NavigatorSettings, SortOption};
use crate::source::Millis;
use crate::state::{Mods, NavigatorState};
use crate::tree::{NavRow, NavRowKind};

/// A pane's header: the tools along its top.
const HEADER: f64 = 36.0;

/// The modifiers of a click, ⌘ on a Mac and Ctrl elsewhere as the command.
pub(super) fn mods(m: Modifiers) -> Mods {
    Mods {
        shift: m.shift(),
        command: if cfg!(target_os = "macos") {
            m.meta()
        } else {
            m.ctrl()
        },
    }
}

/// The folders' pane; `calendar` draws the calendar at its foot.
#[component]
pub(super) fn NavPane(nav: Navigator, on_open: EventHandler<Open>, calendar: bool) -> Element {
    let s = nav.state.read();
    let any = s.any_expanded();
    let reordering = s.reordering;
    let dual = s.settings().dual_pane;
    let calendar_on = s.settings().calendar;
    // Collected, so the state is let go before the header is drawn.
    #[allow(clippy::needless_collect)]
    let header_commands: Vec<crate::template::TemplateCommand> = s
        .templates()
        .commands
        .iter()
        .filter(|c| c.placement == crate::template::CommandPlacement::Header)
        .cloned()
        .collect();
    let calendar_open = s.calendar_open;
    let show_hidden = s.show_hidden();
    let profiles: Vec<(String, String)> = s
        .profiles()
        .all()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();
    let profile = s.profile().id.clone();
    let profile_name = s.profile().name.clone();
    drop(s);
    let row_menu = nav.menu.read().clone();
    let mut profile_menu = use_signal(|| false);
    rsx! {
        div { class: "nv-nav", "data-pane": "navigation",
            style: "position:absolute; inset:0; display:flex; flex-direction:column; background:var(--nv-background);",
            div { class: "nv-header",
                style: "flex:0 0 {HEADER}px; display:flex; align-items:center; gap:2px; padding:0 6px; border-bottom:1px solid var(--nv-divider); position:relative; z-index:4;",
                button { class: "nv-tool", "aria-label": if any { "Collapse all folders" } else { "Expand all folders" },
                    onclick: move |_| nav.update(NavigatorState::collapse_or_expand_all),
                    if any { ChevronsDownUp { size: 16 } } else { ChevronsUpDown { size: 16 } }
                }
                button { class: if reordering { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Reorder the top folders",
                    onclick: move |_| nav.update(|s| s.reordering = !s.reordering),
                    ArrowUpDown { size: 16 }
                }
                div { style: "flex:1;" }
                // The vault profiles (`VaultTitle`'s menu), when there are more than one.
                if profiles.len() > 1 {
                    button { class: if profile_menu() { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Profile: {profile_name}",
                        "data-testid": "nv-profile", title: "{profile_name}",
                        onclick: move |_| profile_menu.toggle(),
                        UserRound { size: 16 }
                    }
                }
                button { class: if show_hidden { "nv-tool nv-on" } else { "nv-tool" },
                    "aria-label": if show_hidden { "Hide hidden items" } else { "Show hidden items" },
                    "data-testid": "nv-show-hidden",
                    onclick: move |_| nav.update(NavigatorState::toggle_hidden),
                    if show_hidden { Eye { size: 16 } } else { EyeOff { size: 16 } }
                }
                if profile_menu() {
                    div { class: "nv-menu nv-profile-menu", role: "menu",
                        style: "position:absolute; top:{HEADER}px; right:6px; z-index:10; min-width:180px; padding:4px; border-radius:8px; \
                                background:var(--nv-background); border:1px solid var(--nv-divider);",
                        MenuHeading { label: "Vault profile" }
                        for (id, name) in profiles {
                            MenuRow { key: "{id}", data: format!("profile:{id}"), label: name, checked: id == profile,
                                on_pick: move |()| {
                                    if let Err(e) = nav.update(|s| s.select_profile(&id)) { nav.say(e); }
                                    profile_menu.set(false);
                                } }
                        }
                        MenuRow { data: "profiles:edit".to_owned(), label: "Edit profiles…".to_owned(), checked: false,
                            on_pick: move |()| {
                                profile_menu.set(false);
                                if !super::profiles_page::open_profiles_page() {
                                    nav.say("No settings page to edit the profiles on");
                                }
                            } }
                    }
                }
                // "Create note" commands placed in the header
                // (`templateCommands[].placement`).
                for c in header_commands {
                    {
                        let id = c.id.clone();
                        let icon = crate::Icon::parse(if c.icon.is_empty() { "file-plus" } else { &c.icon });
                        rsx! {
                            button { key: "{c.id}", class: "nv-tool nv-template-command", "aria-label": "{c.name}", title: "{c.name}",
                                onclick: move |_| nav.run_template_command(&id, on_open),
                                if let Some(i) = icon { super::icon::IconView { nav, icon: i, size: 16, colour: None } }
                            }
                        }
                    }
                }
                if calendar_on {
                    button { class: if calendar_open { "nv-tool nv-on" } else { "nv-tool" },
                        "aria-label": if calendar_open { "Hide calendar" } else { "Show calendar" },
                        "data-testid": "nv-calendar-toggle",
                        onclick: move |_| nav.update(NavigatorState::toggle_calendar),
                        CalendarDays { size: 16 }
                    }
                }
                button { class: if dual { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Two panes",
                    onclick: move |_| nav.settings.update(|s| s.dual_pane = !s.dual_pane),
                    Columns2 { size: 16 }
                }
            }
            PinnedRows { nav, on_open }
            div { style: "flex:1 1 0; min-height:0; position:relative;",
                VirtualPane { nav, pane: Pane::Navigation, label: "Folders", NavRows { nav, on_open } }
                if let Some(menu) = row_menu {
                    tree_rows::Menu { nav, on_open, menu }
                }
            }
            if calendar {
                Calendar { nav, on_open }
            }
        }
    }
}

/// The shortcuts (and recent notes) pinned over the tree (`pinShortcuts`):
/// they stay as the tree scrolls.
#[component]
fn PinnedRows(nav: Navigator, on_open: EventHandler<Open>) -> Element {
    let s = nav.state.read();
    let rows: Vec<NavRow> = s.pinned_rows().to_vec();
    if rows.is_empty() {
        return rsx! {};
    }
    let h = s.nav_item_height();
    let active = s.active().map(str::to_owned);
    let reordering = s.reordering;
    drop(s);
    #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
    let total = h.mul_add(rows.len() as f64, 6.0);
    rsx! {
        div { class: "nv-pinned", style: "flex:0 0 auto; position:relative; height:{total}px; border-bottom:1px solid var(--nv-divider);",
            for (i, row) in rows.into_iter().enumerate() {
                {
                    #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
                    let top = h.mul_add(i as f64, 3.0);
                    match row.kind.clone() {
                        NavRowKind::Section { id, name, expanded, count } => rsx! {
                            tree_rows::SectionRow { key: "{row.key}", nav, id, name, expanded, count, top, height: h, drop_here: false }
                        },
                        NavRowKind::Shortcut { at, shortcut, name, missing } => rsx! {
                            tree_rows::ShortcutRow { key: "{row.key}", nav, on_open, at, shortcut, name, missing, reorder: reordering, top, height: h, drop_here: false }
                        },
                        NavRowKind::Recent { path, name } => rsx! {
                            tree_rows::RecentRow { key: "{row.key}", nav, on_open, active: active.as_deref() == Some(path.as_str()), path, name, top, height: h }
                        },
                        _ => rsx! {},
                    }
                }
            }
        }
    }
}

#[component]
fn NavRows(nav: Navigator, on_open: EventHandler<Open>) -> Element {
    let s = nav.state.read();
    let window = s.window(Pane::Navigation);
    let offset = s.scroll(Pane::Navigation).offset;
    let indent = f64::from(s.settings().indent);
    let show_count = s.settings().show_note_count;
    let separate = s.settings().separate_note_counts;
    let selected = s.selection().folder().map(str::to_owned);
    let target = s.target();
    let reordering = s.reordering;
    let root_level = u16::from(s.settings().show_root);
    let tag_root_level = u16::from(s.settings().all_tags_row);
    let over = nav.drag.read().as_ref().and_then(|d| d.over.clone());
    let renaming = nav.renaming.read().clone();
    let search = s.search().clone();
    let active = s.active().map(str::to_owned);
    // Collected, so the state is let go before the rows are drawn.
    #[allow(clippy::needless_collect)]
    let rows: Vec<_> = window
        .filter_map(|i| {
            let row = s.nav_rows().get(i)?;
            let top = s.nav_offsets().start(i) - offset;
            let h = s.nav_offsets().height(i);
            let note = match &row.kind {
                NavRowKind::Folder { path, .. } => s.folder_note(path).is_some(),
                _ => false,
            };
            let colour = match &row.kind {
                NavRowKind::Tag { key, .. } => s.colour(&format!("tag:{key}")).map(str::to_owned),
                NavRowKind::Property {
                    key,
                    value: Some(v),
                    ..
                } => s
                    .colour(&format!("property:{key}={v}"))
                    .or_else(|| s.colour(&format!("property:{key}")))
                    .map(str::to_owned),
                NavRowKind::Property {
                    key, value: None, ..
                } => s.colour(&format!("property:{key}")).map(str::to_owned),
                _ => None,
            };
            // A folder's, tag's or property's icon and background
            // (`folderIcons`, `…BackgroundColors`).
            let (icon, background) = match &row.kind {
                NavRowKind::Folder { path, .. } => (
                    s.folder_icon(path),
                    s.background(&format!("folder:{path}")).map(str::to_owned),
                ),
                NavRowKind::Tag { key, .. } => (
                    s.item_icon(&format!("tag:{key}")),
                    s.background(&format!("tag:{key}")).map(str::to_owned),
                ),
                NavRowKind::Property { key, value, .. } => {
                    let t = value.as_ref().map_or_else(
                        || format!("property:{key}"),
                        |v| format!("property:{key}={v}"),
                    );
                    (s.item_icon(&t), s.background(&t).map(str::to_owned))
                }
                _ => (None, None),
            };
            let mut colour = match &row.kind {
                NavRowKind::Folder { path, .. } => {
                    s.colour(&format!("folder:{path}")).map(str::to_owned)
                }
                _ => colour,
            };
            let mut background = background;
            // The rainbow where the row has no colour of its own
            // (`applyRainbowOverlay`).
            match s.rainbow(&row.key) {
                Some((c, false)) if colour.is_none() => colour = Some(c.to_owned()),
                Some((c, true)) if background.is_none() => {
                    background = Some(format!("color-mix(in srgb, {c} 22%, transparent)"));
                }
                _ => {}
            }
            // A top row's gap (`rootLevelSpacing`) above it, not in it.
            let base = s.nav_item_height();
            let (top, h) = if h > base + 0.5
                && matches!(
                    row.kind,
                    NavRowKind::Folder { .. }
                        | NavRowKind::Tag { .. }
                        | NavRowKind::Property { .. }
                ) {
                (top + h - base, base)
            } else {
                (top, h)
            };
            Some((row.clone(), top, h, note, colour, icon, background))
        })
        .collect();
    let icon_only = nav.state.peek().settings().color_icon_only;
    drop(s);
    rsx! {
        for (row, top, h, note, colour, icon, background) in rows {
            if let NavRowKind::Banner { src } = row.kind.clone() {
                tree_rows::BannerRow { key: "{row.key}", nav, src, top, height: h }
            } else if let NavRowKind::Section { id, name, expanded, count } = row.kind.clone() {
                tree_rows::SectionRow { key: "{row.key}", nav, id, name, expanded, count, top, height: h,
                    drop_here: over.as_deref() == Some(row.key.as_str()) }
            } else if let NavRowKind::Shortcut { at, shortcut, name, missing } = row.kind.clone() {
                tree_rows::ShortcutRow { key: "{row.key}", nav, on_open, at, shortcut, name, missing, reorder: reordering, top, height: h,
                    drop_here: over.as_deref() == Some(row.key.as_str()) }
            } else if let NavRowKind::Recent { path, name } = row.kind.clone() {
                tree_rows::RecentRow { key: "{row.key}", nav, on_open, active: active.as_deref() == Some(path.as_str()), path, name, top, height: h }
            } else if let NavRowKind::Tag { tag, key, name, has_children, expanded, count } = row.kind.clone() {
                ItemRow {
                    key: "{row.key}",
                    nav,
                    on_open,
                    custom_icon: icon,
                    background,
                    icon_only,
                    search: search.tag_match(&tag),
                    selected: target == NavTarget::Tag(tag.clone()),
                    drop_here: over.as_deref() == Some(row.key.as_str()),
                    target: NavTarget::Tag(tag),
                    row_key: row.key.clone(),
                    name,
                    level: row.level,
                    has_children,
                    expanded,
                    count: count_label(count, show_count, separate),
                    icon: match key.as_str() {
                        crate::tags::TAGGED => RowIcon::AllTags,
                        crate::tags::UNTAGGED => RowIcon::Untagged,
                        _ => RowIcon::Tag,
                    },
                    colour,
                    top,
                    height: h,
                    indent,
                    reorder: reordering && row.level == tag_root_level && key != crate::tags::TAGGED,
                }
            } else if let NavRowKind::Property { key, value, name, has_children, expanded, count } = row.kind.clone() {
                ItemRow {
                    key: "{row.key}",
                    nav,
                    on_open,
                    custom_icon: icon,
                    background,
                    icon_only,
                    search: None,
                    selected: target == NavTarget::Property { key: key.clone(), value: value.clone() },
                    drop_here: over.as_deref() == Some(row.key.as_str()),
                    icon: if key == crate::properties::ROOT {
                        RowIcon::AllProperties
                    } else if value.is_some() {
                        RowIcon::Value
                    } else {
                        RowIcon::Key
                    },
                    reorder: reordering && row.level == 1 && value.is_none(),
                    target: NavTarget::Property { key, value },
                    row_key: row.key.clone(),
                    name,
                    level: row.level,
                    has_children,
                    expanded,
                    count: count_label(count, show_count, separate),
                    colour,
                    top,
                    height: h,
                    indent,
                }
            } else if let NavRowKind::Folder { path, name, has_children, expanded, count } = row.kind {
                FolderRow {
                    key: "{row.key}",
                    nav,
                    on_open,
                    note,
                    icon,
                    colour,
                    background,
                    icon_only,
                    drop_here: over.as_deref() == Some(row.key.as_str()),
                    renaming: renaming == Some(Renaming::Folder(path.clone())),
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
    on_open: EventHandler<Open>,
    /// It has a folder note: its name opens it.
    note: bool,
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
    /// Under a drag that it takes.
    drop_here: bool,
    /// Its name is being edited.
    renaming: bool,
    /// Its own icon, colour and background.
    icon: Option<crate::Icon>,
    colour: Option<String>,
    background: Option<String>,
    /// The colour paints the icon alone.
    icon_only: bool,
) -> Element {
    let pad = f64::from(level).mul_add(indent, 6.0);
    let root = path.is_empty();
    let name_colour = colour
        .as_ref()
        .filter(|_| !icon_only)
        .map_or_else(String::new, |c| format!("color:{c};"));
    let icon_colour = colour
        .clone()
        .unwrap_or_else(|| "var(--nv-faint)".to_owned());
    let bg = background
        .filter(|_| !selected && !drop_here)
        .map_or_else(String::new, |b| format!("background:{b};"));
    let class = match (selected, drop_here) {
        (_, true) => "nv-row nv-drop-target",
        (true, false) => "nv-row nv-selected",
        (false, false) => "nv-row",
    };
    let name_weight = if root { 600 } else { 400 };
    let (p1, p2, p3, p4) = (path.clone(), path.clone(), name.clone(), name.clone());
    let (p5, p6, p7, p8) = (path.clone(), path.clone(), path.clone(), path.clone());
    let name_flex = tree_rows::name_flex(nav, !count.is_empty() && !reorder);
    let deck_rows = tree_rows::use_deck_rows();
    let deck_rows_down = deck_rows.clone();
    rsx! {
        div {
            class,
            role: "treeitem",
            "data-folder": "{path}",
            "data-drop-zone": "folder",
            "aria-selected": if selected { "true" } else { "false" },
            "aria-expanded": if has_children { if expanded { "true" } else { "false" } } else { "" },
            "aria-level": "{level}",
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{height}px; display:flex; align-items:center; \
                    gap:4px; padding-left:{pad}px; padding-right:8px; border-radius:6px; white-space:nowrap; {bg}",
            onclick: move |e: MouseEvent| {
                if nav.dragged.cloned() {
                    return;
                }
                // ⌘-click on a folder the app loads as a deck: a new deck.
                if mods(e.modifiers()).command && tree_rows::deck_row(deck_rows.as_ref(), &p1) {
                    tree_rows::open_deck(&p1, on_open);
                    return;
                }
                let out = nav.update(|s| s.click_folder(&p1));
                // The folder note, else (`autoSelectFirstFileOnFocusChange`)
                // its first note.
                let out = out.or_else(|| nav.state.peek().first_note());
                nav.act(out, on_open);
            },
            // Add to shortcuts, hide (`buildFolderMenu`'s).
            oncontextmenu: move |e: MouseEvent| {
                e.prevent_default();
                let mut menu = nav.menu;
                menu.set(Some(tree_rows::RowMenu { target: NavTarget::Folder(p5.clone()), top: top + height }));
            },
            // Dragged onto another folder: moved into it (the root stays).
            onpointerdown: move |e: PointerEvent| {
                if tree_rows::middle_click(&e, deck_rows_down.as_ref(), &p6, on_open) {
                    return;
                }
                if !p6.is_empty() && !renaming {
                    tree_rows::start_drag(nav, crate::state::Drag::Folder(p6.clone()), &e);
                }
            },
            onpointermove: move |e: PointerEvent| tree_rows::drag_move(nav, &e),
            onpointerup: move |_| tree_rows::drag_end(nav, on_open),
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
            span { style: "flex:0 0 auto; display:flex; color:{icon_colour};",
                if let Some(i) = icon {
                    super::icon::IconView { nav, icon: i, size: 15, colour }
                } else if root { Vault { size: 15 } } else if expanded { FolderOpen { size: 15 } } else { Folder { size: 15 } }
            }
            if renaming {
                tree_rows::InlineRename { nav, on_open, what: Renaming::Folder(p7), value: name }
            } else {
                span { class: if note { "nv-name nv-has-note" } else { "nv-name" },
                    "data-folder-note": if note { "true" } else { "false" },
                    title: tree_rows::tooltip(nav, &p8),
                    style: "flex:{name_flex}; min-width:0; overflow:hidden; text-overflow:ellipsis; font-weight:{name_weight}; {name_colour}",
                    "{name}"
                }
            }
            if note {
                span { class: "nv-folder-note", "aria-label": "Has a folder note", style: "flex:0 0 auto; display:flex; color:var(--nv-faint);", FileText { size: 12 } }
            }
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
                tree_rows::Leader { nav }
                span { class: "nv-count", style: "flex:0 0 auto; color:var(--nv-faint); font-size:11px;", "{count}" }
            }
            tree_rows::Guides { nav, level, indent }
        }
    }
}

#[component]
pub(super) fn ListPane(
    nav: Navigator,
    on_open: EventHandler<Open>,
    on_context_menu: Option<EventHandler<(String, MouseEvent)>>,
    single: bool,
    on_new_note: Option<EventHandler<String>>,
    /// The calendar at its foot: the panes are stacked.
    calendar: bool,
) -> Element {
    let s = nav.state.read();
    let target = s.target();
    let title = match &target {
        NavTarget::Tag(t) if t == crate::tags::TAGGED => "Tags".to_owned(),
        NavTarget::Tag(t) if t == crate::tags::UNTAGGED => "Untagged".to_owned(),
        NavTarget::Tag(t) => format!("#{t}"),
        NavTarget::Folder(f) => s.index().folder_name(f).to_owned(),
        NavTarget::Property { key, .. } if key == crate::properties::ROOT => {
            "Properties".to_owned()
        }
        NavTarget::Property { key, value } => {
            let name = s
                .property_tree()
                .key(key)
                .map_or_else(|| key.clone(), |k| k.name.clone());
            match value {
                Some(v) => {
                    let shown = s
                        .property_tree()
                        .value(key, v)
                        .map_or_else(|| v.clone(), |n| n.name.clone());
                    format!("{name}: {shown}")
                }
                None => name,
            }
        }
    };
    let shown = s.effective().clone();
    let has_own = s.appearance(&target).is_some();
    // New note goes in the folder on show (the root for a tag).
    let new_in = match &target {
        NavTarget::Folder(f) => f.clone(),
        _ => String::new(),
    };
    let empty = s.list_order().is_empty();
    let writable = nav.source().can_write();
    let searching = s.search_open || !s.search().text().is_empty();
    drop(s);
    let own = *nav.own.read() || has_own;
    let place = match &target {
        NavTarget::Tag(_) => "this tag",
        NavTarget::Folder(_) => "this folder",
        NavTarget::Property { .. } => "this property",
    };
    let notice = nav.notice.read().clone();
    let mut menu = use_signal(|| false);
    let compact = shown.mode == ListMode::Compact;
    let descendants = shown.include_descendants;
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
                // Notebook Navigator's "New note" in the list's header: in
                // the folder on show.
                // The app's, when it gave one; else the navigator's own,
                // from the folder's template.
                if on_new_note.is_some() || writable {
                    button { class: "nv-tool", "aria-label": "New note", "data-testid": "nv-new-note",
                        onclick: move |_| match on_new_note {
                            Some(new_note) => new_note.call(new_in.clone()),
                            None => nav.new_note(crate::state::NewNote { folder: new_in.clone(), ..Default::default() }, on_open),
                        },
                        SquarePen { size: 16 }
                    }
                }
                button { class: if searching { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Search", "data-testid": "nv-search-toggle",
                    onclick: move |_| nav.update(NavigatorState::toggle_search),
                    Search { size: 16 }
                }
                button { class: if descendants { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Notes from folders below",
                    onclick: move |_| nav.edit_list(|_, s| s.include_descendants = !s.include_descendants),
                    ListTree { size: 16 }
                }
                button { class: if compact { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Compact rows",
                    onclick: move |_| nav.edit_list(|_, s| s.mode = if s.mode == ListMode::Compact { ListMode::Standard } else { ListMode::Compact }),
                    Rows3 { size: 16 }
                }
                button { class: if menu() { "nv-tool nv-on" } else { "nv-tool" }, "aria-label": "Sort and group",
                    onclick: move |_| menu.toggle(),
                    ArrowDownWideNarrow { size: 16 }
                }
                if menu() {
                    div { class: "nv-menu", role: "menu",
                        style: "position:absolute; top:{HEADER}px; right:6px; z-index:10; min-width:230px; padding:4px; border-radius:8px; \
                                background:var(--nv-background); border:1px solid var(--nv-divider);",
                        MenuHeading { label: "Sort by" }
                        for option in SortOption::ALL {
                            MenuRow { key: "{option.id()}", data: format!("sort:{}", option.id()), label: option.label(), checked: option == shown.sort,
                                on_pick: move |()| { nav.edit_list(move |_, s| s.sort = option); menu.set(false); } }
                        }
                        MenuHeading { label: "Group by" }
                        for g in Grouping::ALL {
                            MenuRow { key: "group-{g.id()}", data: format!("group:{}", g.id()),
                                label: if g == Grouping::Property && !shown.group_property.trim().is_empty() { format!("Property: {}", shown.group_property.trim()) } else { g.label().to_owned() },
                                checked: g == shown.grouping,
                                on_pick: move |()| { nav.edit_list(move |_, s| s.grouping = g); menu.set(false); } }
                        }
                        MenuHeading { label: "Applies to" }
                        MenuRow { key: "own", data: "own".to_owned(), label: format!("Only {place}"), checked: own,
                            on_pick: move |()| {
                                if own {
                                    let target = nav.state.peek().target();
                                    nav.update(|s| s.set_appearance(&target, crate::Appearance::default()));
                                    let mut o = nav.own;
                                    o.set(false);
                                } else {
                                    let mut o = nav.own;
                                    o.set(true);
                                }
                            } }
                    }
                }
            }
            tree_rows::SearchBar { nav }
            if let Some(text) = notice {
                div { class: "nv-notice", role: "status",
                    style: "flex:0 0 auto; display:flex; align-items:center; gap:6px; padding:4px 10px; font-size:12px; color:var(--nv-muted); border-bottom:1px solid var(--nv-divider);",
                    span { style: "flex:1;", "{text}" }
                    button { class: "nv-tool", style: "width:18px; height:18px;", "aria-label": "Dismiss",
                        onclick: move |_| { let mut n = nav.notice; n.set(None); }, "×" }
                }
            }
            div { style: "flex:1 1 0; min-height:0; position:relative;",
                if empty {
                    div { style: "position:absolute; top:0; left:0; right:0; bottom:0; display:flex; flex-direction:column; justify-content:center; padding:16px;",
                        p { style: "margin:0; text-align:center; color:var(--nv-faint); font-size:12px;",
                            if searching { "No notes match the search" } else { "No notes here" }
                        }
                    }
                } else {
                    VirtualPane { nav, pane: Pane::Files, label: "Notes", ListRows { nav, on_open, on_context_menu } }
                }
            }
            if calendar {
                Calendar { nav, on_open }
            }
        }
    }
}

#[component]
fn MenuHeading(label: &'static str) -> Element {
    rsx! {
        div { class: "nv-menu-heading", style: "padding:6px 8px 2px; font-size:11px; font-weight:600; color:var(--nv-faint);", "{label}" }
    }
}

#[component]
fn MenuRow(data: String, label: String, checked: bool, on_pick: EventHandler<()>) -> Element {
    rsx! {
        div { class: "nv-menu-row", role: "menuitemradio",
            "data-menu": "{data}",
            "aria-checked": if checked { "true" } else { "false" },
            style: "display:flex; align-items:center; gap:6px; height:26px; padding:0 8px; border-radius:5px; white-space:nowrap;",
            onclick: move |_| on_pick.call(()),
            span { style: "flex:0 0 14px; display:flex;", if checked { Check { size: 13 } } }
            "{label}"
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
    let settings = s.effective().clone();
    let clock = s.clock();
    let active = s.active().map(str::to_owned);
    let target = s.target();
    let source = nav.source();
    // The search's words, marked in the names they matched (the filter
    // search's; the full text's are in the text).
    let marked: Vec<String> = if s.settings().search_provider == crate::SearchProvider::Filter {
        s.search().words().to_vec()
    } else {
        Vec::new()
    };
    // Collected, so the state is let go before the rows are drawn.
    #[allow(clippy::needless_collect)]
    let rows: Vec<_> = window
        .filter_map(|i| {
            let row = s.list_rows().get(i)?;
            let top = s.list_offsets().start(i) - offset;
            let h = s.list_offsets().height(i);
            match &row.kind {
                ListRowKind::Header {
                    label,
                    group,
                    collapsed,
                    count,
                    pinned,
                } => Some(Item::Header {
                    key: row.key.clone(),
                    label: label.clone(),
                    group: group.clone(),
                    collapsed: *collapsed,
                    count: settings.show_group_counts.then_some(*count),
                    pinned: *pinned,
                    top,
                    h,
                }),
                ListRowKind::File {
                    path,
                    parent,
                    pinned,
                    lines,
                    thumb,
                    ..
                } => {
                    let f = s.file(path)?;
                    let at = list::date_of(f, &settings, clock);
                    let cx = s.list_context(&target);
                    let tags = list::shown_tags(f, &cx)
                        .into_iter()
                        .map(|(full, shown)| Pill {
                            search: s.search().tag_match(full),
                            key: full.to_owned(),
                            text: shown.to_owned(),
                            colour: settings
                                .color_tags
                                .then(|| s.colour(&format!("tag:{full}")).map(str::to_owned))
                                .flatten(),
                        })
                        .collect::<Vec<_>>();
                    let mut tags = tags;
                    if settings.color_tags {
                        // Coloured tags first (`prioritizeColoredFileTags`).
                        tags.sort_by_key(|t| t.colour.is_none());
                    }
                    let mut properties: Vec<Pill> = if settings.show_properties {
                        list::shown_properties(f, &settings)
                            .into_iter()
                            .map(|(k, v)| Pill {
                                search: None,
                                colour: s
                                    .colour(&format!("property:{k}={v}"))
                                    .or_else(|| s.colour(&format!("property:{k}")))
                                    .map(str::to_owned),
                                key: k,
                                text: v,
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let words = f.words.map(|w| format!("{} words", thousands(w)));
                    if let Some(w) = &words
                        && settings.text_count_placement == CountPlacement::Property
                    {
                        properties.push(Pill {
                            key: "words".to_owned(),
                            text: w.clone(),
                            colour: None,
                            search: None,
                        });
                    }
                    let image = thumb.as_ref().map(|t| match t {
                        Thumb::File(p) => source
                            .resource_url(p)
                            .map_or_else(|| ThumbView::Badge(extension_badge(p)), ThumbView::Url),
                        Thumb::Url(u) => ThumbView::Url(u.clone()),
                        Thumb::Badge(b) => ThumbView::Badge(b.clone()),
                    });
                    let name = f.title(&settings.name_fields()).to_owned();
                    let at_file = format!("file:{path}");
                    Some(Item::File(FileItem {
                        icon: s.file_icon(path),
                        colour: s.colour(&at_file).map(str::to_owned),
                        background: s.background(&at_file).map(str::to_owned),
                        icon_only: s.settings().color_icon_only,
                        path: path.clone(),
                        marks: crate::search::highlights(&name, &marked),
                        name,
                        // The line the full text matched, else the preview.
                        preview: s
                            .excerpt(path)
                            .map_or_else(|| f.preview.clone().unwrap_or_default(), str::to_owned),
                        preview_marks: s
                            .excerpt(path)
                            .map(|e| crate::search::highlights(e, s.search().words()))
                            .unwrap_or_default(),
                        date: date_label(at, clock),
                        parent: parent.clone(),
                        selected: s.selection().is_selected(path),
                        open: active.as_deref() == Some(path.as_str()),
                        pinned: *pinned,
                        lines: *lines,
                        tags: if lines.tags { tags } else { Vec::new() },
                        properties: if lines.properties {
                            properties
                        } else {
                            Vec::new()
                        },
                        tasks: f
                            .tasks
                            .filter(|_| lines.tasks)
                            .map(|t| (t.total.saturating_sub(t.unfinished), t.total)),
                        words: words.filter(|_| lines.count_in_title),
                        image,
                        top,
                        h,
                    }))
                }
                _ => None,
            }
        })
        .collect();
    drop(s);
    rsx! {
        for item in rows {
            match item {
                Item::Header { key, label, group, collapsed, count, pinned, top, h } => rsx! {
                    div { key: "{key}", class: "nv-group", role: "heading",
                        "data-group": "{group}",
                        "aria-expanded": if collapsed { "false" } else { "true" },
                        style: "position:absolute; left:0; right:0; top:{top}px; height:{h}px; display:flex; align-items:flex-end; gap:4px; \
                                padding:0 10px 5px; color:var(--nv-heading); font-size:12px; font-weight:600; cursor:default;",
                        onclick: move |_| nav.update(|s| s.toggle_group(&group)),
                        span { class: "nv-group-chevron", "aria-label": if collapsed { "Expand group" } else { "Collapse group" },
                            style: "display:flex; color:var(--nv-faint);",
                            if collapsed { ChevronRight { size: 12 } } else { ChevronDown { size: 12 } }
                        }
                        if pinned { span { style: "display:flex;", Pin { size: 11 } } }
                        span { "{label}" }
                        if let Some(n) = count { span { class: "nv-group-count", style: "font-weight:400; color:var(--nv-faint);", "({n})" } }
                    }
                },
                Item::File(f) => rsx! {
                    FileRow {
                        key: "{f.path}",
                        nav,
                        on_open,
                        on_context_menu,
                        item: f,
                        settings: settings.clone(),
                    }
                },
            }
        }
    }
}

/// `n` with thousands separated: `12,345`.
fn thousands(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && digits.len().saturating_sub(i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A file's kind as a badge: `PDF`, `SVG`.
fn extension_badge(path: &str) -> String {
    path.rsplit_once('.')
        .map_or_else(String::new, |(_, e)| e.to_ascii_uppercase())
}

enum Item {
    Header {
        key: String,
        label: String,
        group: String,
        collapsed: bool,
        count: Option<usize>,
        pinned: bool,
        top: f64,
        h: f64,
    },
    File(FileItem),
}

#[derive(Clone, PartialEq)]
struct Pill {
    /// The tag (whole) or property key.
    key: String,
    text: String,
    colour: Option<String>,
    /// A tag the search names (`searchMatchTerms`).
    search: Option<crate::SearchMatch>,
}

#[derive(Clone, PartialEq)]
enum ThumbView {
    Url(String),
    Badge(String),
}

#[derive(Clone, PartialEq)]
struct FileItem {
    path: String,
    name: String,
    /// Where the search's words are in it (byte ranges).
    marks: Vec<std::ops::Range<usize>>,
    /// Its icon (its own, its name's or its type's), colour and
    /// background (`fileIcons`, `fileColors`, `fileBackgroundColors`).
    icon: Option<crate::Icon>,
    colour: Option<String>,
    background: Option<String>,
    icon_only: bool,
    preview: String,
    /// The search's words in the preview (a full-text match's line).
    preview_marks: Vec<std::ops::Range<usize>>,
    date: String,
    parent: Option<String>,
    selected: bool,
    open: bool,
    pinned: bool,
    lines: Lines,
    tags: Vec<Pill>,
    properties: Vec<Pill>,
    /// Done of all.
    tasks: Option<(u32, u32)>,
    /// Its count, beside its name.
    words: Option<String>,
    image: Option<ThumbView>,
    top: f64,
    h: f64,
}

#[component]
fn FileRow(
    nav: Navigator,
    on_open: EventHandler<Open>,
    on_context_menu: Option<EventHandler<(String, MouseEvent)>>,
    item: FileItem,
    settings: NavigatorSettings,
) -> Element {
    let deck_rows = tree_rows::use_deck_rows();
    let deck_rows_down = deck_rows.clone();
    let FileItem {
        path,
        name,
        marks,
        icon,
        colour,
        background,
        icon_only,
        preview,
        preview_marks,
        date,
        parent,
        selected,
        open,
        pinned,
        lines,
        tags,
        properties,
        tasks,
        words,
        image,
        top,
        h,
    } = item;
    let compact = settings.mode == ListMode::Compact;
    let class = if selected {
        "nv-row nv-file nv-selected"
    } else {
        "nv-row nv-file"
    };
    let (p1, p2, p3) = (path.clone(), path.clone(), path.clone());
    let renaming = nav.renaming.read().as_ref() == Some(&Renaming::File(path.clone()));
    let name_colour = colour
        .as_ref()
        .filter(|_| !icon_only)
        .map_or_else(|| "var(--nv-text)".to_owned(), Clone::clone);
    let bg = background
        .filter(|_| !selected)
        .map_or_else(String::new, |b| format!("background:{b};"));
    let p4 = path.clone();
    let title_h = f64::from(lines.title) * 20.0;
    let preview_h = f64::from(lines.preview) * 18.0;
    let weight = if open { 700 } else { 600 };
    let (justify, padding) = if compact {
        ("center", "0 10px")
    } else {
        ("flex-start", "8px 10px")
    };
    let wrap = if lines.title == 1 { "nowrap" } else { "normal" };
    // An image fills the row's height less its padding, 42 to the size set.
    let size = f64::from(settings.feature_image_size)
        .min(h - 16.0)
        .max(list::IMAGE_FLOOR);
    let image_w = if settings.force_square_image {
        format!("{size}px")
    } else {
        "auto".to_owned()
    };
    let parent_shown = settings.show_parent_folder && !pinned;
    rsx! {
        div {
            class,
            role: "option",
            "data-file": "{path}",
            "data-pinned": if pinned { "true" } else { "false" },
            "aria-selected": if selected { "true" } else { "false" },
            "aria-current": if open { "page" } else { "false" },
            style: "position:absolute; left:4px; right:4px; top:{top}px; height:{h}px; border-radius:6px; overflow:hidden; \
                    display:flex; flex-direction:row; gap:8px; padding:{padding}; box-sizing:border-box; {bg}",
            onclick: move |e: MouseEvent| {
                if nav.dragged.cloned() {
                    return;
                }
                let m = mods(e.modifiers());
                // ⌘-click on a row the app loads as a deck: a new deck.
                if m.command && tree_rows::deck_row(deck_rows.as_ref(), &p1) {
                    tree_rows::open_deck(&p1, on_open);
                    return;
                }
                let out = nav.state.clone().write().click_file(&p1, m);
                nav.act(out, on_open);
            },
            oncontextmenu: move |e: MouseEvent| {
                if let Some(menu) = on_context_menu {
                    menu.call((p2.clone(), e));
                }
            },
            // Dragged onto a tag or a property: the selected notes when it
            // is one of them, else it alone.
            onpointerdown: move |e: PointerEvent| {
                if renaming || tree_rows::middle_click(&e, deck_rows_down.as_ref(), &p3, on_open) {
                    return;
                }
                let paths: Vec<String> = {
                    let s = nav.state.peek();
                    if s.selection().is_selected(&p3) {
                        s.selection().files().iter().cloned().collect()
                    } else {
                        vec![p3.clone()]
                    }
                };
                tree_rows::start_drag(nav, crate::state::Drag::Notes(paths), &e);
            },
            onpointermove: move |e: PointerEvent| tree_rows::drag_move(nav, &e),
            onpointerup: move |_| tree_rows::drag_end(nav, on_open),
            div { class: "nv-file-body", style: "flex:1 1 auto; min-width:0; display:flex; flex-direction:column; justify-content:{justify};",
                div { class: "nv-file-title", style: "flex:0 0 auto; display:flex; align-items:baseline; gap:6px; min-width:0;",
                    if let Some(i) = icon {
                        span { class: "nv-file-icon", style: "flex:0 0 auto; align-self:center; display:flex; color:{colour.clone().unwrap_or_else(|| \"var(--nv-faint)\".to_owned())};",
                            super::icon::IconView { nav, icon: i, size: 14, colour: colour.clone() }
                        }
                    }
                    if renaming {
                        tree_rows::InlineRename { nav, on_open, what: Renaming::File(p4), value: name }
                    } else {
                        div { class: "nv-file-name",
                            title: tree_rows::tooltip(nav, &path),
                            style: "flex:1 1 auto; min-width:0; max-height:{title_h}px; line-height:20px; overflow:hidden; text-overflow:ellipsis; \
                                    white-space:{wrap}; font-weight:{weight}; color:{name_colour};",
                            if marks.is_empty() {
                                "{name}"
                            } else {
                                for (i, (text, mark)) in marked_parts(&name, &marks).into_iter().enumerate() {
                                    if mark {
                                        mark { key: "{i}", class: "nv-search-highlight", "{text}" }
                                    } else {
                                        span { key: "{i}", "{text}" }
                                    }
                                }
                            }
                        }
                    }
                    if let Some(w) = words {
                        span { class: "nv-file-count", style: "flex:0 0 auto; font-size:11px; color:var(--nv-faint);", "{w}" }
                    }
                }
                if lines.preview > 0 && !preview.trim().is_empty() {
                    div { class: "nv-file-preview",
                        style: "flex:0 0 auto; max-height:{preview_h}px; line-height:18px; overflow:hidden; color:var(--nv-muted); font-size:12px;",
                        if preview_marks.is_empty() {
                            "{preview}"
                        } else {
                            for (i, (text, mark)) in marked_parts(&preview, &preview_marks).into_iter().enumerate() {
                                if mark {
                                    mark { key: "{i}", class: "nv-search-highlight", "{text}" }
                                } else {
                                    span { key: "{i}", "{text}" }
                                }
                            }
                        }
                    }
                }
                if lines.meta {
                    div { class: "nv-file-second-line",
                        style: "flex:0 0 auto; height:19px; display:flex; align-items:center; gap:8px; white-space:nowrap; overflow:hidden; color:var(--nv-faint); font-size:12px;",
                        if lines.date { span { class: "nv-file-date", "{date}" } }
                        if let Some((done, total)) = tasks {
                            span { class: "nv-file-tasks", "aria-label": "{done} of {total} tasks done", style: "display:flex; align-items:center; gap:4px;",
                                span { style: "position:relative; width:28px; height:4px; border-radius:2px; background:var(--nv-divider); overflow:hidden; display:block;",
                                    span { style: "position:absolute; left:0; top:0; bottom:0; width:{progress(done, total)}%; background:var(--nv-open); display:block;" }
                                }
                                "{done}/{total}"
                            }
                        }
                        if parent_shown && let Some(p) = parent {
                            span { class: "nv-file-parent", style: "display:flex; align-items:center; gap:3px; overflow:hidden; text-overflow:ellipsis;",
                                Folder { size: 11 }
                                "{p}"
                            }
                        }
                    }
                }
                if !tags.is_empty() {
                    div { class: "nv-file-tags", style: "flex:0 0 26px; display:flex; align-items:center; gap:4px; overflow:hidden; white-space:nowrap;",
                        for t in tags {
                            TagPill { key: "{t.key}", nav, pill: t }
                        }
                    }
                }
                if !properties.is_empty() {
                    div { class: "nv-file-properties", style: "flex:0 0 26px; display:flex; align-items:center; gap:4px; overflow:hidden; white-space:nowrap;",
                        for p in properties {
                            PropertyPill { key: "{p.key}", nav, pill: p }
                        }
                    }
                }
            }
            if let Some(img) = image {
                div { class: "nv-thumb-box", style: "flex:0 0 auto; align-self:center; display:flex; align-items:center; justify-content:center; height:{size}px; width:{image_w}; min-width:{size}px; border-radius:6px; overflow:hidden; background:var(--nv-thumb);",
                    match img {
                        // Fit, not cover: a wide image (a label, a banner) shows whole in
                        // its box rather than as its middle (#442: "hitect l").
                        ThumbView::Url(u) => rsx! { img { class: "nv-feature-image", src: "{u}", alt: "", style: "width:100%; height:100%; object-fit:contain; display:block;" } },
                        ThumbView::Badge(b) => {
                            // A long kind (`CANVAS`, `EXCALIDRAW`) smaller, to fit the box.
                            let font = if b.len() > 4 { 8 } else { 10 };
                            rsx! { span { class: "nv-badge", style: "font-size:{font}px; font-weight:700; color:var(--nv-faint); overflow:hidden; max-width:100%; text-align:center;", "{b}" } }
                        }
                    }
                }
            }
        }
    }
}

/// `text` cut at `marks`: each part, and whether it is marked.
fn marked_parts(text: &str, marks: &[std::ops::Range<usize>]) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut at = 0;
    for m in marks {
        if m.start > at {
            out.push((text.get(at..m.start).unwrap_or("").to_owned(), false));
        }
        out.push((text.get(m.clone()).unwrap_or("").to_owned(), true));
        at = m.end;
    }
    if at < text.len() {
        out.push((text.get(at..).unwrap_or("").to_owned(), false));
    }
    out
}

/// Done over all, as a percentage.
fn progress(done: u32, total: u32) -> f64 {
    if total == 0 {
        0.0
    } else {
        f64::from(done) * 100.0 / f64::from(total)
    }
}

fn pill_style(colour: Option<&str>) -> String {
    let base = "display:inline-flex; align-items:center; gap:3px; height:20px; padding:0 7px; border-radius:10px; font-size:11px; flex:0 0 auto;";
    colour.map_or_else(
        || format!("{base} background:var(--nv-pill); color:var(--nv-muted);"),
        |c| format!("{base} background:color-mix(in srgb, {c} 22%, transparent); color:{c};"),
    )
}

/// A tag on a row: a click goes to the tag (`navigateToTag`).
#[component]
fn TagPill(nav: Navigator, pill: Pill) -> Element {
    let tag = pill.key.clone();
    let class = match pill.search {
        Some(crate::SearchMatch::Include) => "nv-pill nv-tag nv-search-include",
        Some(crate::SearchMatch::Exclude) => "nv-pill nv-tag nv-search-exclude",
        None => "nv-pill nv-tag",
    };
    rsx! {
        span { class, "data-tag": "{pill.key}", role: "link",
            style: pill_style(pill.colour.as_deref()),
            onclick: move |e: MouseEvent| {
                e.stop_propagation();
                let m = tree_rows::click_mods(&e);
                if m.command {
                    nav.update(|s| s.click_tag(&tag, m));
                } else {
                    nav.update(|s| s.select_tag(&tag));
                }
            },
            Tag { size: 10 }
            "{pill.text}"
        }
    }
}

/// A property on a row: a click lists the notes with that value; with
/// ⌘/Ctrl, it goes into the search (`useFileItemPills`).
#[component]
fn PropertyPill(nav: Navigator, pill: Pill) -> Element {
    let (key, value) = (pill.key.clone(), pill.text.clone());
    // A note's count shown with its properties is not one.
    let is_property = !key.starts_with("words");
    rsx! {
        span { class: "nv-pill nv-property", "data-property": "{pill.key}", role: if is_property { "link" } else { "" },
            style: pill_style(pill.colour.as_deref()),
            onclick: move |e: MouseEvent| {
                if !is_property {
                    return;
                }
                e.stop_propagation();
                let m = tree_rows::click_mods(&e);
                if m.command {
                    nav.update(|s| s.click_property(&key, Some(&value), m));
                } else {
                    nav.update(|s| {
                        s.select_property(&key, Some(&value));
                        s.focus = Pane::Files;
                    });
                }
            },
            "{pill.text}"
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
