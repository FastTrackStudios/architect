//! The toolbars in the window (feature `ui`).
//!
//! A pane of their own (a dock's, a float, a window) or a pane's header, each drawn by
//! [`ToolbarStrip`] from the toolbar in force under its key (a person's
//! copy over an extension's), its buttons following their actions' toggle
//! and armed state live (`docs/content/architecture/extensions.md`,
//! "Toolbars").

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::{ActionList, Invocation, InvocationSource};
use architect_shell::{ShellTheme, named_icon};
use architect_workspace::{ContextEntry, HeaderTools, PaneId, Side, Target, View, try_use_pane_of};
use dioxus::prelude::*;
use futures::StreamExt as _;
use futures::channel::mpsc;
use serde_json::json;

use crate::host::Host;
use crate::interface::{ExtensionUi, ExtensionView};
use crate::toolbar::{MenuItem, MenuKind, ToolColors, ToolItem, ToolKind, ToolbarPlace, preset};
use crate::toolbar_edit::At;
use crate::toolbar_editor::{Editor, InsertMenu, Kept, Press, item_name, keep_in};
use crate::toolbars::Toolbar;

/// A pane toolbar's workspace kind (and its pane's id): `toolbar:<key>`.
fn pane_kind(key: &str) -> String {
    format!("toolbar:{key}")
}

/// The toolbars that are panes of their own, as workspace views.
pub(crate) fn pane_views(ui: ExtensionUi) -> Vec<View> {
    pane_toolbars(&ui.host())
        .into_iter()
        .map(|(t, place)| {
            let key = t.key.clone();
            let home = match place {
                ToolbarPlace::Dock(side) => Target::Dock(dock_side(&side)),
                _ => Target::Tab,
            };
            View::new(pane_kind(&t.key), t.decl.title.clone(), move |_| {
                rsx! { ToolbarStrip { ui, toolbar: key.clone() } }
            })
            .home(home)
            .headless()
        })
        .collect()
}

fn pane_toolbars(host: &Host) -> Vec<(Toolbar, ToolbarPlace)> {
    host.toolbars()
        .all(host.contributions())
        .into_iter()
        .filter_map(|t| {
            let place = t.decl.place()?;
            place.is_pane().then_some((t, place))
        })
        .collect()
}

fn dock_side(side: &str) -> Side {
    match side {
        "left" => Side::Left,
        "bottom" => Side::Bottom,
        _ => Side::Right,
    }
}

/// Provides the workspace's [`HeaderTools`]: a pane header's toolbars
/// (`pane`, `view:<kind>`).
pub(crate) fn provide_header_tools(ui: ExtensionUi) {
    use_context_provider(move || {
        HeaderTools::new(move |kind: &str, pane: &PaneId| {
            let (_, _) = (ui.revision(), ui.ticks());
            let host = ui.host();
            let bars = host.toolbars().in_header_of(host.contributions(), kind);
            if bars.is_empty() {
                return None;
            }
            let (pane, kind) = (pane.0.clone(), kind.to_owned());
            Some(rsx! {
                for t in bars {
                    ToolbarStrip {
                        key: "{t.key}",
                        ui,
                        toolbar: t.key,
                        pane: pane.clone(),
                        view: kind.clone(),
                        header: true,
                    }
                }
            })
        })
    });
}

/// Opens each pane toolbar where it goes as it appears (once a window:
/// one the person closed stays closed until it appears again), and closes
/// its pane when it goes.
pub(crate) fn use_pane_toolbars(ui: ExtensionUi, app: Arc<ActionList>) {
    let placed = use_hook(|| {
        Rc::new(std::cell::RefCell::new(
            std::collections::BTreeSet::<String>::new(),
        ))
    });
    use_effect(move || {
        let (_, _) = (ui.revision(), ui.ticks());
        let now: BTreeMap<String, (String, ToolbarPlace)> = pane_toolbars(&ui.host())
            .into_iter()
            .map(|(t, place)| (pane_kind(&t.key), (t.decl.title, place)))
            .collect();
        // The app's views follow the set (a new toolbar of the person's).
        let listed: Vec<(String, String, String)> = now
            .iter()
            .map(|(k, (title, place))| (k.clone(), title.clone(), format!("{place:?}")))
            .collect();
        let mut panes = ui.toolbar_panes;
        if *panes.peek() != listed {
            panes.set(listed);
        }
        let gone: Vec<String> = placed
            .borrow()
            .iter()
            .filter(|k| !now.contains_key(*k))
            .cloned()
            .collect();
        for kind in gone {
            placed.borrow_mut().remove(&kind);
            let args = json!({ "panes": [kind] }).to_string();
            let app = Arc::clone(&app);
            spawn(async move {
                let _ = app.run_json_async("WORKSPACE_CLOSE", args.as_bytes()).await;
            });
        }
        for (kind, (title, place)) in now {
            if placed.borrow().contains(&kind) {
                continue;
            }
            placed.borrow_mut().insert(kind.clone());
            let at = match place {
                ToolbarPlace::Dock(side) => json!({ "dock": { "side": side } }),
                ToolbarPlace::Window => json!({ "window": {} }),
                _ => {
                    json!({ "float": { "rect": { "x": 160.0, "y": 96.0, "w": 360.0, "h": 64.0 } } })
                }
            };
            let args = json!({ "view": kind, "id": kind, "title": title, "at": at }).to_string();
            let app = Arc::clone(&app);
            let still = Rc::clone(&placed);
            spawn(async move {
                // Its view joins the app's views as the app draws again,
                // and the workspace's actions are there only while a
                // workspace is on show: until then it waits (while it is
                // still there).
                let mut wait = 50_u64;
                while still.borrow().contains(&kind) {
                    if app
                        .run_json_async("WORKSPACE_OPEN", args.as_bytes())
                        .await
                        .is_ok()
                    {
                        return;
                    }
                    futures_timer::Delay::new(std::time::Duration::from_millis(wait)).await;
                    wait = wait.saturating_mul(2).min(1000);
                }
            });
        }
    });
}

/// The toolbars' own rows of a context menu: on a toolbar (`toolbar`,
/// its key), Edit toolbars and its settings; on an item while editing
/// (`toolbar-item`, `<key>#<at>`), its settings, grouping, removing. Each
/// is a `TOOLBAR_*` action, run told the target.
pub(crate) fn context_rows(host: &Host, target: &str, id: &str) -> Vec<ContextEntry> {
    let row = |label: &str, command: &str| ContextEntry {
        label: label.to_owned(),
        command: command.to_owned(),
        checked: None,
    };
    let bars = host.toolbars();
    match target {
        "toolbar" => vec![
            ContextEntry {
                checked: Some(bars.editing()),
                ..row("Edit toolbars", "TOOLBAR_EDIT")
            },
            row("Toolbar settings…", "TOOLBAR_SELECT"),
        ],
        "toolbar-item" => {
            let Some((key, at)) = id.split_once('#') else {
                return Vec::new();
            };
            let Ok(at) = At::parse(at) else {
                return Vec::new();
            };
            let item = bars
                .get(host.contributions(), key)
                .and_then(|t| crate::toolbar_edit::get(&t.decl, at).cloned());
            let mut out = vec![row("Settings…", "TOOLBAR_SELECT")];
            match (item.map(|i| i.kind), at.inner) {
                (Some(ToolKind::Group), _) => out.push(row("Ungroup", "TOOLBAR_UNGROUP")),
                (Some(ToolKind::Button), None) => {
                    out.push(row("Group with the next", "TOOLBAR_GROUP"));
                }
                _ => {}
            }
            out.push(row("Remove", "TOOLBAR_REMOVE"));
            out
        }
        _ => Vec::new(),
    }
}

/// A signal that moves when anything happens on the host's list (an
/// action ran, the app's state changed), when the toolbars change (a file
/// read again, an action armed), and as a person's toolbar files are
/// looked at again (once a second). Natively a thread waits on the list's
/// events; on the web the list is asked a few times a second.
pub(crate) fn use_ticks(host: &Host) -> Signal<u64> {
    let mut ticks = use_signal(|| 0_u64);
    let host = host.clone();
    use_future(move || {
        let host = host.clone();
        async move {
            let (tx, mut rx) = mpsc::unbounded::<()>();
            let bars = host.toolbars().clone();
            let heard = tx.clone();
            let _held = bars.subscribe(move |_| {
                let _ = heard.unbounded_send(());
            });
            let list = host.list().clone();
            // From now: what it still keeps is not news.
            let mut since = latest(&list);
            #[cfg(not(target_family = "wasm"))]
            {
                let tx = tx.clone();
                let spawned = std::thread::Builder::new()
                    .name("ext-toolbar-ticks".to_owned())
                    .spawn(move || {
                        while !tx.is_closed() {
                            let (seen, next) =
                                list.events_since(since, 256, std::time::Duration::from_secs(1));
                            since = next;
                            if !seen.is_empty() && tx.unbounded_send(()).is_err() {
                                return;
                            }
                        }
                    });
                if spawned.is_err() {
                    return;
                }
            }
            // A person's files, looked at again (and, on the web, the
            // list's events).
            spawn(async move {
                while !tx.is_closed() {
                    let pause = if cfg!(target_family = "wasm") {
                        250
                    } else {
                        1000
                    };
                    futures_timer::Delay::new(std::time::Duration::from_millis(pause)).await;
                    #[cfg(target_family = "wasm")]
                    {
                        let (seen, next) = list.events_since(since, 256, std::time::Duration::ZERO);
                        since = next;
                        if !seen.is_empty() && tx.unbounded_send(()).is_err() {
                            return;
                        }
                    }
                    // A change is heard through the subscription.
                    let _ = bars.poll();
                }
            });
            while rx.next().await.is_some() {
                // Several at once are one redraw.
                while rx.try_recv().is_ok() {}
                let n = ticks.peek().wrapping_add(1);
                ticks.set(n);
            }
        }
    });
    ticks
}

/// The sequence number of the last event `list` keeps (0 for none).
fn latest(list: &ActionList) -> u64 {
    let mut since = 0;
    loop {
        let (seen, next) = list.events_since(since, 4096, std::time::Duration::ZERO);
        if seen.is_empty() {
            return since;
        }
        since = next;
    }
}

/// A colour of a toolbar file: a theme token's name (`accent`) as its CSS
/// variable, a CSS colour as it is; `fallback` when unset.
fn colour(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_owned()
    } else if value.starts_with('#') || value.contains('(') {
        value.to_owned()
    } else {
        format!("var(--{value}, {fallback})")
    }
}

/// What every item of a toolbar is drawn with.
#[derive(Clone, PartialEq)]
struct Ctx {
    ui: ExtensionUi,
    /// The toolbar's key and owner.
    key: String,
    owner: String,
    /// Where it is (`dock:right`, `view:note`): an invocation's target.
    at: String,
    pane: Option<String>,
    view: Option<String>,
    header: bool,
    hide_labels: bool,
    /// A button's height, its corners, the space between items, points.
    size: u32,
    rounding: u32,
    gap: u32,
    /// An icon's size, points.
    glyph: usize,
    /// The colours every button starts from (its preset's).
    preset: ToolColors,
    /// Where its relative image files are: the person's folder, or its
    /// extension's package.
    images: Option<std::path::PathBuf>,
    /// The editor is on.
    editing: bool,
    /// The dropdown open now, by its item's path (`3`, `2.1`).
    open: Signal<Option<String>>,
}

impl Ctx {
    fn invocation(&self) -> Invocation {
        let mut i =
            Invocation::new(InvocationSource::Toolbar).on(self.at.clone(), self.key.clone());
        i.pane.clone_from(&self.pane);
        i.view.clone_from(&self.view);
        i
    }

    fn run(&self, action: &str) {
        if let Some(c) = try_consume_context::<architect_commands::Commands>() {
            let _ = c.run_invoked(action, None, self.invocation());
        }
    }

    /// The pane it is in: the header's, or its own.
    fn pane_id(&self) -> PaneId {
        PaneId(self.pane.clone().unwrap_or_else(|| pane_kind(&self.key)))
    }
}

/// A toolbar, by its key: its items in rows (a `newline` item starts
/// another), each button running its action told where it was pressed
/// (source `toolbar`, target its place, id its key, the pane), a toggle's
/// showing on (`aria-pressed`), an armed one armed (`data-armed`).
///
/// While the toolbars are edited (`Toolbars::editing`) it shows a **+**
/// chip between its items and at its end, its items pressed open their
/// settings and dragged move (`crate::toolbar_editor`).
#[allow(clippy::too_many_lines)]
#[component]
pub fn ToolbarStrip(
    ui: ExtensionUi,
    /// The toolbar's key (`fts.metronome/transport`, `user/mine`).
    toolbar: String,
    /// The pane whose header holds it.
    #[props(default)]
    pane: Option<String>,
    /// That pane's view kind.
    #[props(default)]
    view: Option<String>,
    /// In a pane's header (compact) rather than a pane of its own.
    #[props(default)]
    header: bool,
) -> Element {
    let (_, _) = (ui.revision(), ui.ticks());
    let open = use_signal(|| None::<String>);
    let t = crate::window_ui::shell_theme();
    let host = ui.host();
    let editor = try_use_context::<Editor>();
    // Outside a workspace (over a phone's keyboard) it has no pane, and
    // no context menu of one.
    let handle = try_use_pane_of(PaneId(pane.clone().unwrap_or_else(|| pane_kind(&toolbar))));
    let Some(bar) = host.toolbars().get(host.contributions(), &toolbar) else {
        return rsx! {};
    };
    let at = if header {
        view.as_ref()
            .map_or_else(|| bar.decl.at.clone(), |v| format!("view:{v}"))
    } else {
        bar.decl.at.clone()
    };
    let style = &bar.decl.style;
    let editing = editor.is_some() && host.toolbars().editing();
    let images = if bar.file.is_some() || bar.owner == crate::toolbars::PERSON {
        host.toolbars().dir()
    } else {
        host.get(&bar.owner).and_then(|l| l.dir)
    };
    let cx = Ctx {
        ui,
        key: bar.key.clone(),
        owner: bar.owner.clone(),
        at,
        pane,
        view,
        header,
        hide_labels: bar.decl.hide_labels,
        size: style.size.unwrap_or(if header { 24 } else { 28 }),
        rounding: style.rounding.unwrap_or(6),
        gap: style.spacing.unwrap_or(4),
        glyph: style.size.map_or(16, |s| {
            usize::try_from(s.saturating_mul(4) / 7).unwrap_or(16)
        }),
        preset: preset(&style.preset),
        images,
        editing,
        open,
    };
    // Rows: a `newline` item starts the next.
    let mut rows: Vec<Vec<(usize, ToolItem)>> = vec![Vec::new()];
    for (n, item) in bar.decl.items.iter().enumerate() {
        if item.newline && rows.last().is_some_and(|r| !r.is_empty()) {
            rows.push(Vec::new());
        }
        if let Some(row) = rows.last_mut() {
            row.push((n, item.clone()));
        }
    }
    let count = bar.decl.items.len();
    let last_row = rows.len().saturating_sub(1);
    let outer = if header {
        "display:flex; flex-direction:column; gap:2px; flex:0 0 auto;".to_owned()
    } else {
        format!(
            "flex:1 1 0; display:flex; flex-direction:column; gap:{}px; padding:8px; \
             background:{}; overflow:visible;",
            cx.gap, t.sheet
        )
    };
    let key = bar.key.clone();
    let (mount_key, menu_key) = (key.clone(), key.clone());
    let empty = count == 0 && !editing && !header;
    let title = bar.decl.title.clone();
    let settings_host = host;
    let settings_key = key.clone();
    rsx! {
        div {
            role: "toolbar",
            "aria-label": "{bar.decl.title}",
            "data-toolbar": "{bar.key}",
            "data-editing": editing.then_some("true"),
            style: "{outer}",
            onmounted: move |e| {
                if let Some(ed) = editor {
                    ed.mount(&mount_key, "", e.data());
                }
            },
            onpointermove: move |e: PointerEvent| {
                if let Some(ed) = editor {
                    let p = e.data().client_coordinates();
                    ed.moved(narrow(p.x), narrow(p.y));
                }
            },
            // Its empty space: the `toolbar` target's menu (Edit toolbars,
            // its settings, what extensions add).
            oncontextmenu: move |e: MouseEvent| {
                if let Some(h) = &handle {
                    h.context_menu("toolbar", menu_key.clone(), &e);
                }
            },
            if empty {
                span { style: "font-size:11px; color:{t.ink_3};", "An empty toolbar: Edit toolbars to fill it." }
            }
            for (r, row) in rows.into_iter().enumerate() {
                div { key: "{r}", style: "display:flex; flex-wrap:wrap; align-items:center; gap:{cx.gap}px;",
                    if editing && r == 0 {
                        button {
                            "data-toolbar-settings": "{key}",
                            title: "{title}: its settings",
                            style: "flex:0 0 auto; height:{cx.size.min(24)}px; padding:0 6px; border-radius:6px; \
                                    border:1px solid {t.rule}; background:transparent; color:{t.ink_3}; font-size:11px; cursor:pointer;",
                            onclick: {
                                let (h, k) = (settings_host.clone(), settings_key.clone());
                                move |_| h.toolbars().select(Some((k.clone(), None)))
                            },
                            "⚙ {title}"
                        }
                    }
                    for (n, item) in row {
                        if editing {
                            EditableItem { key: "{n}-{item.kind:?}", cx: cx.clone(), item, path: n.to_string(), chip: true }
                        } else {
                            ToolItemView { key: "{n}", cx: cx.clone(), item, path: n.to_string() }
                        }
                    }
                    if editing && r == last_row {
                        InsertChip { key: "c{count}", cx: cx.clone(), at: count.to_string() }
                    }
                }
            }
        }
    }
}

/// A measured length, as points.
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

/// A **+** between items while editing: its menu puts an action, a
/// separator, a widget or a dropdown there.
#[component]
fn InsertChip(cx: Ctx, at: String) -> Element {
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let mut adding = editor.adding;
    let here = (cx.key.clone(), at.clone());
    let is_open = adding().as_ref() == Some(&here);
    let h = cx.size.min(24);
    rsx! {
        div { style: "position:relative; flex:0 0 auto; display:flex;",
            button {
                "data-insert-at": "{cx.key}#{at}",
                "aria-label": "Insert here",
                "aria-expanded": "{is_open}",
                style: "width:16px; height:{h}px; padding:0; border-radius:5px; border:1px dashed {t.rule}; \
                        background:transparent; color:{t.ink_3}; font-size:12px; line-height:1; cursor:pointer;",
                onclick: move |_| {
                    let now = adding.peek().as_ref() == Some(&here);
                    adding.set((!now).then(|| here.clone()));
                },
                "+"
            }
            if is_open {
                InsertMenu { ui: cx.ui, toolbar: cx.key.clone(), at: at.clone() }
            }
        }
    }
}

/// An item while editing: pressed, its settings; dragged, it moves;
/// right-clicked, the `toolbar-item` context menu. A group's buttons are
/// each one too.
#[component]
fn EditableItem(
    cx: Ctx,
    item: ToolItem,
    path: String,
    /// A + chip before it (a top-level item's).
    #[props(default)]
    chip: bool,
) -> Element {
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let handle = try_use_pane_of(cx.pane_id());
    let Ok(at) = At::parse(&path) else {
        return rsx! {};
    };
    let host = cx.ui.host();
    let group = item.kind == ToolKind::Group;
    let label = item_name(&host, &item);
    let selected = host
        .toolbars()
        .selected()
        .is_some_and(|(k, a)| k == cx.key && a.as_deref() == Some(path.as_str()));
    let ring = if selected {
        t.accent.to_string()
    } else {
        t.rule.to_string()
    };
    let mount_at = if group {
        format!("g{path}")
    } else {
        path.clone()
    };
    // Measured while it is here: forgotten as it goes.
    let mut mounted_as = use_hook(|| CopyValue::new((cx.key.clone(), mount_at.clone())));
    mounted_as.set((cx.key.clone(), mount_at.clone()));
    use_drop(move || {
        let (key, at) = mounted_as.cloned();
        let mut mounts = editor.mounts;
        if let Ok(mut m) = mounts.try_write() {
            m.remove(&(key, at));
        }
    });
    let (mk, pk, ck) = (cx.key.clone(), cx.key.clone(), cx.key.clone());
    let menu_id = format!("{ck}#{path}");
    let up_host = host;
    let inner = if group {
        let label = item.label.clone();
        let buttons = item.items;
        rsx! {
            div { role: "group", "aria-label": "{label}", "data-group": "{label}",
                style: "flex:0 0 auto; display:flex; flex-direction:column; align-items:center; gap:2px; padding:2px;",
                div { style: "display:flex; gap:2px; border:1px solid {t.rule}; border-radius:7px;",
                    for (n, b) in buttons.into_iter().enumerate() {
                        EditableItem { key: "{n}", cx: cx.clone(), item: b, path: format!("{path}.{n}") }
                    }
                }
                // Its grip: pressed here, the whole group moves.
                span { "data-group-grip": "{path}", style: "font-size:10px; color:{t.ink_3}; padding:0 6px; cursor:grab;",
                    if label.is_empty() { "Group" } else { "{label}" }
                }
            }
        }
    } else {
        rsx! {
            div { style: "pointer-events:none; display:flex;",
                ToolItemView { cx: cx.clone(), item, path: path.clone() }
            }
        }
    };
    rsx! {
        if chip {
            InsertChip { cx: cx, at: path }
        }
        div {
            "data-edit-item": "{menu_id}",
            "aria-selected": "{selected}",
            style: "position:relative; flex:0 0 auto; display:flex; border-radius:7px; cursor:grab; \
                    outline:1px dashed {ring}; outline-offset:1px; touch-action:none;",
            onmounted: move |e| editor.mount(&mk, &mount_at, e.data()),
            onpointerdown: move |e: PointerEvent| {
                // A right-press is its context menu's.
                if e.data().trigger_button().is_some_and(|b| b != dioxus::html::input_data::MouseButton::Primary) {
                    return;
                }
                e.stop_propagation();
                let p = e.data().client_coordinates();
                editor.pressed(Press { key: pk.clone(), at, x: narrow(p.x), y: narrow(p.y), label: label.clone(), group });
            },
            onpointerup: move |e: PointerEvent| {
                e.stop_propagation();
                editor.released(&up_host);
            },
            oncontextmenu: move |e: MouseEvent| {
                e.stop_propagation();
                if let Some(h) = &handle {
                    h.context_menu("toolbar-item", menu_id.clone(), &e);
                }
            },
            {inner}
        }
    }
}

#[component]
fn ToolItemView(cx: Ctx, item: ToolItem, path: String) -> Element {
    let t = crate::window_ui::shell_theme();
    match item.kind {
        ToolKind::Button => rsx! { ToolButton { cx, item, path, joined: false } },
        ToolKind::Separator => rsx! {
            div { role: "separator", style: "flex:0 0 auto; width:1px; align-self:stretch; min-height:18px; background:{t.rule}; margin:0 2px;" }
        },
        ToolKind::Group => {
            let label = item.label.clone();
            rsx! {
                div { role: "group", "aria-label": "{label}", "data-group": "{label}",
                    style: "flex:0 0 auto; display:flex; flex-direction:column; align-items:center; gap:2px;",
                    div { style: "display:flex; border:1px solid {t.rule}; border-radius:{cx.rounding + 1}px; overflow:visible;",
                        for (n, b) in item.items.into_iter().enumerate() {
                            ToolButton { key: "{n}", cx: cx.clone(), item: b, path: format!("{path}.{n}"), joined: true }
                        }
                    }
                    if !label.is_empty() && !cx.header {
                        span { style: "font-size:10px; color:{t.ink_3};", "{label}" }
                    }
                }
            }
        }
        ToolKind::Widget => {
            let (extension, view) = item.view.split_once('/').map_or_else(
                || (cx.owner.clone(), item.view.clone()),
                |(e, v)| (e.to_owned(), v.to_owned()),
            );
            let w = item.width.unwrap_or(120);
            let h = item.height.unwrap_or_else(|| cx.size.saturating_add(4));
            let name = format!("{extension}/{view}");
            rsx! {
                div { "data-widget": "{name}",
                    style: "flex:0 0 auto; width:{w}px; height:{h}px; display:flex; overflow:hidden; border-radius:{cx.rounding}px;",
                    ExtensionView { ui: cx.ui, extension, view, fill: true }
                }
            }
        }
    }
}

#[allow(clippy::too_many_lines)]
#[component]
fn ToolButton(cx: Ctx, item: ToolItem, path: String, joined: bool) -> Element {
    // Its action's state may have moved: read again with every change.
    let (_, _) = (cx.ui.revision(), cx.ui.ticks());
    let t: ShellTheme = crate::window_ui::shell_theme();
    let viewport = try_use_context::<Editor>().map(|e| e.viewport);
    let kept = use_signal(Kept::default);
    let host = cx.ui.host();
    let info = (!item.action.is_empty())
        .then(|| host.list().info(&item.action))
        .flatten();
    let armed = !item.action.is_empty() && host.toolbars().armed().as_deref() == Some(&item.action);
    let checked = info.as_ref().and_then(|i| i.checked);
    let enabled = info.as_ref().is_none_or(|i| i.enabled);
    let label = if item.label.is_empty() {
        info.as_ref()
            .map(|i| i.display_name.clone())
            .unwrap_or_default()
    } else {
        item.label.clone()
    };
    let dropdown = item.action.is_empty();
    let c = item.colors.over(&cx.preset);
    let (fill, ink) = if armed {
        (
            colour(&c.armed_fill, "transparent"),
            colour(&c.armed_ink, &t.danger),
        )
    } else if checked == Some(true) {
        (
            colour(&c.on_fill, &t.accent_soft),
            colour(&c.on_ink, &t.accent),
        )
    } else {
        (colour(&c.fill, "transparent"), colour(&c.ink, &t.ink_2))
    };
    let ring = if armed {
        format!("box-shadow: inset 0 0 0 1px {ink};")
    } else {
        String::new()
    };
    let image = (!item.image.is_empty())
        .then(|| host.toolbars().image(&item.image, cx.images.as_deref()))
        .flatten();
    let iconic = !item.icon.is_empty() || image.is_some();
    let show_label = !(iconic && (item.hide_label || cx.hide_labels || cx.header));
    let (size, glyph) = (cx.size, cx.glyph);
    let width = if iconic && !show_label {
        format!("width:{size}px;")
    } else {
        String::new()
    };
    let pad = if iconic && !show_label { "0" } else { "0 8px" };
    let radius = if joined { 0 } else { cx.rounding };
    let mut open = cx.open;
    let is_open = open().as_deref() == Some(path.as_str());
    let tool = if dropdown {
        format!("menu:{label}")
    } else {
        item.action.clone()
    };
    let (press, right) = (cx.clone(), cx.clone());
    let (action, rc, here) = (item.action.clone(), item.right_click.clone(), path.clone());
    let has_menu = !item.menu.is_empty();
    let editing = cx.editing;
    let place = kept().css();
    let mark = kept().mark();
    rsx! {
        div { style: "position:relative; flex:0 0 auto; display:flex;",
            // Escape, from the button or a row, closes its dropdown.
            onkeydown: move |e: KeyboardEvent| {
                if is_open && e.key() == Key::Named(NamedKey::Escape) {
                    open.set(None);
                    e.stop_propagation();
                }
            },
            button {
                "data-tool": "{tool}",
                title: "{label}",
                "aria-label": "{label}",
                "aria-pressed": checked.map(|on| on.to_string()),
                "aria-haspopup": has_menu.then_some("menu"),
                "aria-expanded": has_menu.then(|| is_open.to_string()),
                "data-armed": armed.then_some("true"),
                disabled: !enabled,
                style: "{width} height:{size}px; padding:{pad}; display:flex; align-items:center; \
                        justify-content:center; gap:6px; border:none; border-radius:{radius}px; cursor:pointer; \
                        background:{fill}; color:{ink}; font-size:12px; {ring}",
                onclick: move |_| {
                    if editing {
                        return;
                    }
                    if action.is_empty() {
                        let now = open.peek().as_deref() == Some(here.as_str());
                        open.set((!now).then(|| here.clone()));
                    } else {
                        press.run(&action);
                    }
                },
                oncontextmenu: {
                    let (action, here) = (item.action, path);
                    move |e: MouseEvent| {
                        if editing {
                            return;
                        }
                        match rc.as_str() {
                            "" | "none" => return,
                            "menu" => open.set(Some(here.clone())),
                            "arm" => {
                                let bars = right.ui.host().toolbars().clone();
                                if bars.armed().as_deref() == Some(action.as_str()) {
                                    let _ = bars.disarm();
                                } else {
                                    bars.arm(&action);
                                }
                            }
                            other => right.run(other),
                        }
                        e.prevent_default();
                        e.stop_propagation();
                    }
                },
                if let Some(src) = image {
                    img { src: "{src}", alt: "", style: "width:{glyph}px; height:{glyph}px; object-fit:contain;" }
                } else if iconic {
                    {named_icon(&item.icon)(architect_shell::Icon { size: glyph, color: ink.clone().into() })}
                }
                if show_label {
                    span { "{label}" }
                }
                if dropdown && !cx.header {
                    span { style: "font-size:9px; opacity:0.7;", "▾" }
                }
            }
            if is_open && has_menu && !editing {
                // A press anywhere else closes it. Far past every edge:
                // Blitz has no `position: fixed`.
                div {
                    "data-menu-backdrop": "{path}",
                    style: "position:absolute; top:-2000px; left:-4000px; width:8000px; height:8000px; z-index:59;",
                    onclick: move |_| open.set(None),
                }
                div {
                    role: "menu",
                    "data-menu": "{path}",
                    "data-kept": mark,
                    style: "position:absolute; {place} z-index:60; min-width:180px; \
                            display:flex; flex-direction:column; padding:4px; border-radius:8px; \
                            background:{t.pill}; border:1px solid {t.rule}; box-shadow:{t.shadow_float};",
                    // Kept in the window: past its right edge (a toolbar in
                    // the right dock) it moves left; past its bottom it
                    // opens upward.
                    onmounted: move |e| keep_in(e.data(), viewport, kept),
                    ToolMenu { cx, rows: item.menu, depth: 0 }
                }
            }
        }
    }
}

/// A dropdown's rows: actions (ticked while a toggle is on), headings,
/// separators, and submenus that open in place.
#[component]
fn ToolMenu(cx: Ctx, rows: Vec<MenuItem>, depth: usize) -> Element {
    let (_, _) = (cx.ui.revision(), cx.ui.ticks());
    let t = crate::window_ui::shell_theme();
    let mut expanded = use_signal(|| None::<usize>);
    let host = cx.ui.host();
    let indent = depth.saturating_mul(10);
    rsx! {
        for (n, row) in rows.into_iter().enumerate() {
            match row.kind {
                MenuKind::Heading => rsx! {
                    div { key: "{n}", style: "padding:6px 8px 2px {8 + indent}px; font-size:11px; font-weight:600; color:{t.ink_3};", "{row.label}" }
                },
                MenuKind::Separator => rsx! {
                    div { key: "{n}", role: "separator", style: "height:1px; margin:4px 6px; background:{t.rule};" }
                },
                MenuKind::Submenu => {
                    let on = expanded() == Some(n);
                    rsx! {
                        button { key: "{n}",
                            role: "menuitem",
                            "aria-expanded": "{on}",
                            style: "display:flex; justify-content:space-between; padding:0 8px 0 {8 + indent}px; min-height:26px; \
                                    border:none; border-radius:5px; background:transparent; color:{t.ink}; font-size:12px; cursor:pointer;",
                            onclick: move |_| expanded.set((!on).then_some(n)),
                            span { "{row.label}" }
                            span { if on { "▾" } else { "›" } }
                        }
                        if on {
                            ToolMenu { key: "{n}-in", cx: cx.clone(), rows: row.items, depth: depth.saturating_add(1) }
                        }
                    }
                }
                MenuKind::Action => {
                    let info = host.list().info(&row.action);
                    let checked = info.as_ref().and_then(|i| i.checked);
                    let label = if row.label.is_empty() {
                        info.map_or_else(|| row.action.clone(), |i| i.display_name)
                    } else {
                        row.label.clone()
                    };
                    let tick = if checked == Some(true) { "✓" } else { "" };
                    let (run, action) = (cx.clone(), row.action.clone());
                    let mut open = cx.open;
                    rsx! {
                        button { key: "{n}",
                            role: if checked.is_some() { "menuitemcheckbox" } else { "menuitem" },
                            "aria-checked": checked.map(|on| on.to_string()),
                            "data-tool": "{row.action}",
                            style: "display:flex; align-items:center; gap:6px; padding:0 8px 0 {indent}px; min-height:26px; \
                                    border:none; border-radius:5px; background:transparent; color:{t.ink}; font-size:12px; \
                                    text-align:left; cursor:pointer;",
                            onclick: move |_| {
                                open.set(None);
                                run.run(&action);
                            },
                            span { style: "flex:0 0 auto; width:14px; text-align:center; color:{t.accent};", "{tick}" }
                            span { "{label}" }
                        }
                    }
                }
            }
        }
    }
}
