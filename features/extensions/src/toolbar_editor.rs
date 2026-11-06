//! The toolbar editor in the window (feature `ui`, #313), modelled on
//! Fergler Advanced Toolbars' edit mode (its design only).
//!
//! **Edit toolbars** (`TOOLBAR_EDIT`, or a toolbar's context menu) turns
//! it on: every toolbar shows a **+** chip between its items (an action, a
//! separator, a widget or a dropdown goes in there), an item pressed opens
//! its settings, an item dragged moves (a button into or out of a group,
//! a whole group, into another toolbar on show; Escape, or a drop off any
//! toolbar, leaves it where it was), and a right-click shows the
//! `toolbar-item` context menu (its own rows and extensions'). An action
//! is found by the palette's own search (`architect_commands::rank`), a
//! widget by its extension, with a live preview.
//!
//! [`ToolbarEditorLayer`] draws all of it over the window (`ExtensionOverlays`
//! mounts it); [`Editor`] is what the toolbars in the window share. Every
//! edit is the host's (`Toolbars::edit`, `crate::toolbar_edit`), written
//! [`DEBOUNCE`] after the last one; the same edits are the `TOOLBAR_*`
//! actions.

use std::collections::BTreeMap;
use std::rc::Rc;

use architect_commands::{Command, Recents, rank};
use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::host::Host;
use crate::host_ui_view::EDITOR_LAYER;
use crate::interface::{ExtensionUi, ExtensionView};
use crate::toolbar::{
    MenuItem, MenuKind, PRESETS, RIGHT_CLICKS, SIZES, ToolItem, ToolKind, ToolbarDecl,
};
use crate::toolbar_edit::{self as edit, At};
use crate::toolbars::DEBOUNCE;

/// A rectangle in window points.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    #[must_use]
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    fn contains(&self, x: f32, y: f32, slack: f32) -> bool {
        x >= self.x - slack
            && x <= self.x + self.w + slack
            && y >= self.y - slack
            && y <= self.y + self.h + slack
    }

    const fn mid_x(&self) -> f32 {
        self.w.mul_add(0.5, self.x)
    }

    const fn mid_y(&self) -> f32 {
        self.h.mul_add(0.5, self.y)
    }
}

/// One item of a toolbar on show, measured.
#[derive(Clone, Debug, PartialEq)]
pub struct Placed {
    pub at: At,
    pub rect: Rect,
    /// It is a group (a button may go into it).
    pub group: bool,
}

/// A toolbar on show, measured: what a drag drops into.
#[derive(Clone, Debug, PartialEq)]
pub struct Strip {
    pub key: String,
    pub rect: Rect,
    pub items: Vec<Placed>,
}

/// Where a dragged item goes: a toolbar, a place in it, and the line
/// that shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub key: String,
    pub at: At,
    pub line: Rect,
}

/// How far outside a toolbar a drop still lands in it, points.
pub const DROP_SLACK: f32 = 8.0;

/// How far a press moves before it is a drag, points.
pub const DRAG_AFTER: f32 = 4.0;

/// Where an item dragged to `(x, y)` goes among `strips`.
///
/// Into a group under the pointer (unless it is a group itself), before the item of
/// the row under the pointer whose middle is past it, or after the row's
/// last. None off every toolbar (the drag is let go of where it was).
#[must_use]
pub fn drop_slot(strips: &[Strip], x: f32, y: f32, moving_group: bool) -> Option<Slot> {
    let strip = strips.iter().find(|s| s.rect.contains(x, y, DROP_SLACK))?;
    let line_at = |r: &Rect, right: bool| {
        let lx = if right { r.x + r.w + 1.0 } else { r.x - 2.0 };
        Rect::new(lx, r.y, 2.0, r.h)
    };
    let mut tops: Vec<&Placed> = strip
        .items
        .iter()
        .filter(|p| p.at.inner.is_none())
        .collect();
    tops.sort_by_key(|p| p.at.index);
    if !moving_group && let Some(g) = tops.iter().find(|p| p.group && p.rect.contains(x, y, 0.0)) {
        let mut inside: Vec<&Placed> = strip
            .items
            .iter()
            .filter(|p| p.at.index == g.at.index && p.at.inner.is_some())
            .collect();
        inside.sort_by_key(|p| p.at.inner);
        let j = inside.iter().filter(|b| b.rect.mid_x() < x).count();
        let line = inside.get(j).map_or_else(
            || inside.last().map_or(g.rect, |b| line_at(&b.rect, true)),
            |b| line_at(&b.rect, false),
        );
        return Some(Slot {
            key: strip.key.clone(),
            at: At::within(g.at.index, j),
            line,
        });
    }
    // The row under the pointer, or the nearest.
    let in_row = |p: &&Placed| y >= p.rect.y && y <= p.rect.y + p.rect.h;
    let mut row: Vec<&Placed> = tops.iter().copied().filter(in_row).collect();
    if row.is_empty()
        && let Some(near) = tops.iter().min_by(|a, b| {
            (a.rect.mid_y() - y)
                .abs()
                .total_cmp(&(b.rect.mid_y() - y).abs())
        })
    {
        let ny = near.rect.mid_y();
        row = tops
            .iter()
            .copied()
            .filter(|p| ny >= p.rect.y && ny <= p.rect.y + p.rect.h)
            .collect();
    }
    let Some(last) = row.last() else {
        let r = strip.rect;
        return Some(Slot {
            key: strip.key.clone(),
            at: At::top(0),
            line: Rect::new(r.x + 6.0, r.y + 4.0, 2.0, (r.h - 8.0).max(8.0)),
        });
    };
    let (at, line) = row.iter().find(|p| p.rect.mid_x() > x).map_or_else(
        || {
            (
                At::top(last.at.index.saturating_add(1)),
                line_at(&last.rect, true),
            )
        },
        |p| (At::top(p.at.index), line_at(&p.rect, false)),
    );
    Some(Slot {
        key: strip.key.clone(),
        at,
        line,
    })
}

/// A press on an item, not yet a drag.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Press {
    pub key: String,
    pub at: At,
    pub x: f32,
    pub y: f32,
    pub label: String,
    pub group: bool,
}

/// An item being dragged.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Drag {
    pub from: Press,
    pub x: f32,
    pub y: f32,
    /// The toolbars on show, measured as it started.
    pub strips: Vec<Strip>,
    pub over: Option<Slot>,
}

/// What a picker's choice is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum For {
    /// A new item at a place.
    Insert { key: String, at: At },
    /// A new row at the end of a dropdown's menu.
    MenuRow { key: String, at: At },
    /// What a right-click runs.
    RightClick { key: String, at: At },
}

/// A picker on show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Picking {
    Action(For),
    Widget(For),
}

/// What the toolbars in a window share while they are edited. Cheap to
/// copy; provided by `use_extension_ui`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Editor {
    /// Each toolbar's and item's element, to measure: by key and place
    /// (`""` the toolbar itself).
    pub(crate) mounts: Signal<BTreeMap<(String, String), Rc<MountedData>>>,
    pub(crate) press: Signal<Option<Press>>,
    pub(crate) drag: Signal<Option<Drag>>,
    /// The + chip whose menu is open: a toolbar's key, a place.
    pub(crate) adding: Signal<Option<(String, String)>>,
    pub(crate) picking: Signal<Option<Picking>>,
    /// The editor's layer over the window, measured when a menu opens:
    /// what the menu keeps inside.
    pub(crate) viewport: Signal<Option<Rc<MountedData>>>,
    /// The last edit refused, and why.
    pub(crate) problem: Signal<Option<String>>,
}

/// Provides the window's [`Editor`].
pub(crate) fn provide_editor() -> Editor {
    let editor = Editor {
        mounts: use_signal(BTreeMap::new),
        press: use_signal(|| None),
        drag: use_signal(|| None),
        adding: use_signal(|| None),
        picking: use_signal(|| None),
        viewport: use_signal(|| None),
        problem: use_signal(|| None),
    };
    use_context_provider(|| editor)
}

/// A measured length, as points.
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

/// Measures `m`, in window points (asked again a few times: Blitz's
/// document is busy while events stream in).
// A UI thread's future: its element is the window's.
#[allow(clippy::future_not_send)]
pub(crate) async fn measure(m: &Rc<MountedData>) -> Option<Rect> {
    for _ in 0..8 {
        if let Ok(r) = m.get_client_rect().await {
            return Some(Rect::new(
                narrow(r.origin.x),
                narrow(r.origin.y),
                narrow(r.size.width),
                narrow(r.size.height),
            ));
        }
        futures_timer::Delay::new(std::time::Duration::from_millis(4)).await;
    }
    None
}

fn pointer(e: &PointerEvent) -> (f32, f32) {
    let p = e.data().client_coordinates();
    (narrow(p.x), narrow(p.y))
}

impl Editor {
    /// Remembers element `m` of toolbar `key` (its place, or `""`).
    pub(crate) fn mount(self, key: &str, at: &str, m: Rc<MountedData>) {
        let mut mounts = self.mounts;
        mounts.write().insert((key.to_owned(), at.to_owned()), m);
    }

    /// Edits toolbar `key` on `host` with `f`, written [`DEBOUNCE`] after
    /// the last edit; a refusal is shown in the edit bar.
    pub(crate) fn apply<T>(
        self,
        host: &Host,
        key: &str,
        f: impl FnOnce(&mut ToolbarDecl) -> Result<T, String>,
    ) -> Option<T> {
        let mut problem = self.problem;
        match host.toolbars().edit(host.contributions(), key, f) {
            Ok((out, n)) => {
                problem.set(None);
                write_later(host, n);
                Some(out)
            }
            Err(why) => {
                problem.set(Some(why));
                None
            }
        }
    }

    /// A press on item `at` of toolbar `key`.
    pub(crate) fn pressed(self, press: Press) {
        let mut p = self.press;
        p.set(Some(press));
    }

    /// The pointer at `(x, y)` over a toolbar: a press moved far enough
    /// becomes a drag, its toolbars measured.
    pub(crate) fn moved(self, x: f32, y: f32) {
        if self.drag.peek().is_some() {
            return;
        }
        let Some(press) = self.press.peek().clone() else {
            return;
        };
        if (x - press.x).hypot(y - press.y) < DRAG_AFTER {
            return;
        }
        let mut drag = self.drag;
        drag.set(Some(Drag {
            from: press,
            x,
            y,
            strips: Vec::new(),
            over: None,
        }));
        let mounts = self.mounts.peek().clone();
        spawn(async move {
            let strips = measure_strips(&mounts).await;
            let mut drag = drag;
            if let Some(d) = drag.write().as_mut() {
                d.over = drop_slot(&strips, d.x, d.y, d.from.group);
                d.strips = strips;
            }
        });
    }

    /// Let go without moving: a press (its item's settings).
    pub(crate) fn released(self, host: &Host) {
        if self.drag.peek().is_some() {
            return;
        }
        let mut press = self.press;
        if let Some(p) = press.take() {
            host.toolbars()
                .select(Some((p.key.clone(), Some(p.at.to_string()))));
        }
    }
}

#[allow(clippy::future_not_send)]
async fn measure_strips(mounts: &BTreeMap<(String, String), Rc<MountedData>>) -> Vec<Strip> {
    let mut strips: BTreeMap<String, Strip> = BTreeMap::new();
    for ((key, at), m) in mounts {
        let Some(rect) = measure(m).await else {
            continue;
        };
        // An element gone from the document measures as nothing.
        if rect.w <= 0.0 && rect.h <= 0.0 {
            continue;
        }
        let strip = strips.entry(key.clone()).or_insert_with(|| Strip {
            key: key.clone(),
            rect: Rect::default(),
            items: Vec::new(),
        });
        if at.is_empty() {
            strip.rect = rect;
        } else if let Some(at_g) = at.strip_prefix('g') {
            if let Ok(at) = At::parse(at_g) {
                strip.items.push(Placed {
                    at,
                    rect,
                    group: true,
                });
            }
        } else if let Ok(at) = At::parse(at) {
            strip.items.push(Placed {
                at,
                rect,
                group: false,
            });
        }
    }
    strips.into_values().filter(|s| s.rect.w > 0.0).collect()
}

/// Writes the host's edits [`DEBOUNCE`] after edit `n`, unless another
/// came since.
pub(crate) fn write_later(host: &Host, n: u64) {
    let bars = host.toolbars().clone();
    spawn(async move {
        futures_timer::Delay::new(DEBOUNCE).await;
        let _ = bars.flush_if(n);
    });
}

/// What an item is called in the editor: its label, its action's name,
/// its kind.
pub(crate) fn item_name(host: &Host, item: &ToolItem) -> String {
    if !item.label.is_empty() {
        return item.label.clone();
    }
    match item.kind {
        ToolKind::Separator => "Separator".to_owned(),
        ToolKind::Group => "Group".to_owned(),
        ToolKind::Widget => item.view.clone(),
        ToolKind::Button => host
            .list()
            .info(&item.action)
            .map_or_else(|| item.action.clone(), |i| i.display_name),
    }
}

/// The theme tokens a colour is picked from.
pub const COLOR_TOKENS: [&str; 12] = [
    "primary",
    "secondary",
    "accent",
    "muted",
    "card",
    "destructive",
    "foreground",
    "background",
    "chart-1",
    "chart-2",
    "chart-3",
    "chart-4",
];

/// The places a toolbar can be, as the settings offer them.
const PLACES: [(&str, &str); 8] = [
    ("dock:right", "Right dock"),
    ("dock:left", "Left dock"),
    ("dock:bottom", "Bottom dock"),
    ("float", "Floating"),
    ("window", "Window"),
    ("pane", "Every pane's header"),
    ("typing", "Over a phone's keyboard"),
    ("hidden", "Hidden"),
];

/// Everything the editor draws over the window: the edit bar, a drag's
/// cover, the settings, the pickers. Mount it once at the root
/// (`ExtensionOverlays` does).
#[component]
pub fn ToolbarEditorLayer(ui: ExtensionUi) -> Element {
    let (_, _) = (ui.revision(), ui.ticks());
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let host = ui.host();
    let bars = host.toolbars().clone();
    let editing = bars.editing();
    let mut viewport = editor.viewport;
    let selected = bars.selected();
    let picking = (editor.picking)();
    let drag = (editor.drag)();
    rsx! {
        div {
            "data-toolbar-editor": if editing { "on" } else { "off" },
            style: "position:absolute; left:0; top:0; right:0; bottom:0; pointer-events:none; z-index:{EDITOR_LAYER};",
            onmounted: move |e| viewport.set(Some(e.data())),
            if editing {
                EditBar { ui }
            }
            if let Some((key, at)) = selected.filter(|_| editing) {
                SettingsSheet { ui, toolbar: key, at }
            }
            if let Some(p) = picking {
                Picker { ui, picking: p }
            }
            if let Some(d) = drag {
                DragCover { ui, drag: d }
            }
        }
    }
}

#[component]
fn EditBar(ui: ExtensionUi) -> Element {
    // The toolbars move under it: read again with every change.
    let (_, _) = (ui.revision(), ui.ticks());
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let host = ui.host();
    let problem = (editor.problem)();
    let (new_host, done_host) = (host.clone(), host);
    let mut problem_sig = editor.problem;
    let pill = format!(
        "border:none; border-radius:6px; padding:0 10px; height:26px; font-size:12px; cursor:pointer; \
         background:{}; color:{};",
        t.accent, t.accent_ink
    );
    let plain = format!(
        "border:1px solid {}; border-radius:6px; padding:0 10px; height:26px; font-size:12px; cursor:pointer; \
         background:transparent; color:{};",
        t.rule, t.ink
    );
    rsx! {
        div {
            role: "toolbar",
            "aria-label": "Toolbar editor",
            "data-testid": "toolbar-edit-bar",
            style: "position:absolute; bottom:44px; left:50%; margin-left:-170px; width:340px; pointer-events:auto; \
                    display:flex; flex-direction:column; gap:4px; padding:6px 8px; border-radius:10px; \
                    background:{t.pill}; border:1px solid {t.rule}; box-shadow:{t.shadow_float}; \
                    font-family:{t.font}; color:{t.ink};",
            div { style: "display:flex; align-items:center; gap:6px;",
                span { style: "flex:1 1 0; font-size:12px; font-weight:600;", "Editing toolbars" }
                button {
                    "data-testid": "toolbar-new",
                    style: "{plain}",
                    onclick: move |_| {
                        let bars = new_host.toolbars();
                        match bars.create(new_host.contributions(), None, "Toolbar", "dock:right") {
                            Ok(key) => {
                                problem_sig.set(None);
                                bars.select(Some((key, None)));
                            }
                            Err(why) => problem_sig.set(Some(why)),
                        }
                    },
                    "New toolbar"
                }
                button {
                    "data-testid": "toolbar-edit-done",
                    style: "{pill}",
                    onclick: move |_| {
                        let bars = done_host.toolbars();
                        let _ = bars.flush();
                        bars.set_editing(false);
                    },
                    "Done"
                }
            }
            if let Some(why) = problem {
                span { role: "alert", style: "font-size:11px; color:{t.danger};", "{why}" }
            }
        }
    }
}

/// Over the window while an item is dragged: it follows the pointer, a
/// line shows where it goes; let go, it moves there; Escape (or a drop
/// off every toolbar) leaves it where it was.
#[component]
fn DragCover(ui: ExtensionUi, drag: Drag) -> Element {
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let host = ui.host();
    let mut d = editor.drag;
    let mut press = editor.press;
    let line = drag.over.as_ref().map(|s| s.line);
    let over = drag
        .over
        .as_ref()
        .map(|s| format!("{}#{}", s.key, s.at))
        .unwrap_or_default();
    let (gx, gy) = (drag.x + 12.0, drag.y + 12.0);
    rsx! {
        div {
            "data-testid": "toolbar-drag",
            tabindex: "0",
            style: "position:absolute; left:0; top:0; right:0; bottom:0; pointer-events:auto; z-index:5; \
                    cursor:grabbing; touch-action:none; outline:none;",
            onmounted: move |e| {
                let m = e.data();
                spawn(async move {
                    let _ = m.set_focus(true).await;
                });
            },
            onpointermove: move |e| {
                let (x, y) = pointer(&e);
                if let Some(now) = d.write().as_mut() {
                    now.x = x;
                    now.y = y;
                    now.over = drop_slot(&now.strips, x, y, now.from.group);
                }
            },
            onpointerup: move |_| {
                press.set(None);
                let Some(done) = d.take() else {
                    return;
                };
                if let Some(slot) = done.over {
                    let bars = host.toolbars();
                    let mut problem = editor.problem;
                    match bars.move_between(host.contributions(), &done.from.key, done.from.at, &slot.key, slot.at) {
                        Ok(_) => {
                            problem.set(None);
                            write_later(&host, bars.last_edit());
                        }
                        Err(why) => problem.set(Some(why)),
                    }
                }
            },
            onkeydown: move |e| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    e.stop_propagation();
                    press.set(None);
                    d.set(None);
                }
            },
            if let Some(l) = line {
                div {
                    "data-testid": "toolbar-drop-line",
                    "data-over": "{over}",
                    style: "position:absolute; left:{l.x}px; top:{l.y}px; width:{l.w}px; height:{l.h}px; \
                            border-radius:1px; background:{t.accent};",
                }
            }
            div {
                style: "position:absolute; left:{gx}px; top:{gy}px; padding:3px 8px; border-radius:6px; font-size:12px; \
                        background:{t.pill}; color:{t.ink}; border:1px solid {t.rule}; box-shadow:{t.shadow_lift};",
                "{drag.from.label}"
            }
        }
    }
}

/// The insert menu of a + chip: an action, a separator, a widget, a
/// dropdown.
#[component]
pub(crate) fn InsertMenu(ui: ExtensionUi, toolbar: String, at: String) -> Element {
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let Ok(place) = At::parse(&at) else {
        return rsx! {};
    };
    let host = ui.host();
    let row = format!(
        "display:flex; align-items:center; gap:6px; padding:0 10px; min-height:26px; border:none; \
         border-radius:5px; background:transparent; color:{}; font-size:12px; text-align:left; cursor:pointer;",
        t.ink
    );
    let mut adding = editor.adding;
    let mut picking = editor.picking;
    let (k1, k2, k3, k4) = (toolbar.clone(), toolbar.clone(), toolbar.clone(), toolbar);
    let (h1, h2) = (host.clone(), host);
    rsx! {
        Popover { ui, test: "toolbar-insert-menu",
            button { style: "{row}", "data-insert": "action",
                onclick: move |_| {
                    adding.set(None);
                    picking.set(Some(Picking::Action(For::Insert { key: k1.clone(), at: place })));
                },
                "Action…"
            }
            button { style: "{row}", "data-insert": "separator",
                onclick: move |_| {
                    adding.set(None);
                    let item = ToolItem { kind: ToolKind::Separator, ..ToolItem::default() };
                    let _ = editor.apply(&h1, &k2, |d| edit::insert(d, Some(place), item));
                },
                "Separator"
            }
            button { style: "{row}", "data-insert": "widget",
                onclick: move |_| {
                    adding.set(None);
                    picking.set(Some(Picking::Widget(For::Insert { key: k3.clone(), at: place })));
                },
                "Widget…"
            }
            button { style: "{row}", "data-insert": "dropdown",
                onclick: move |_| {
                    adding.set(None);
                    let item = ToolItem {
                        label: "Menu".to_owned(),
                        menu: vec![MenuItem { kind: MenuKind::Heading, label: "Menu".to_owned(), ..MenuItem::default() }],
                        ..ToolItem::default()
                    };
                    if let Some(at) = editor.apply(&h2, &k4, |d| edit::insert(d, Some(place), item)) {
                        h2.toolbars().select(Some((k4.clone(), Some(at.to_string()))));
                    }
                },
                "Dropdown"
            }
        }
    }
}

/// Where a menu under its button goes to stay in the window: `shift`
/// points left (past the window's right edge: a toolbar in the right
/// dock), and upward (past its bottom, with room above).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Kept {
    pub shift: f32,
    pub up: bool,
}

impl Kept {
    /// For a menu at `menu` in a window at `view`.
    #[must_use]
    pub fn of(menu: Rect, view: Rect) -> Self {
        let over = menu.x + menu.w - (view.x + view.w - 4.0);
        let shift = if over > 0.0 {
            over.min((menu.x - view.x).max(0.0))
        } else {
            0.0
        };
        let up = menu.y + menu.h > view.y + view.h - 4.0 && menu.y - menu.h > view.y + 40.0;
        Self { shift, up }
    }

    /// Its CSS: where it is against its button.
    #[must_use]
    pub fn css(self) -> String {
        let edge = if self.up {
            "bottom:calc(100% + 4px);"
        } else {
            "top:calc(100% + 4px);"
        };
        if self.shift > 0.0 {
            format!("{edge} left:-{}px;", self.shift.round())
        } else {
            format!("{edge} left:0;")
        }
    }

    /// What `data-kept` says of it (none when it is where it opened).
    #[must_use]
    pub fn mark(self) -> Option<String> {
        (self.shift > 0.0 || self.up).then(|| {
            format!(
                "{}{}",
                if self.shift > 0.0 {
                    format!("left {}", self.shift.round())
                } else {
                    String::new()
                },
                if self.up { " up" } else { "" }
            )
            .trim()
            .to_owned()
        })
    }
}

/// Keeps the menu `m` in the window `viewport`: measured once it is laid
/// out, `kept` set to where it goes.
pub(crate) fn keep_in(
    m: Rc<MountedData>,
    viewport: Option<Signal<Option<Rc<MountedData>>>>,
    mut kept: Signal<Kept>,
) {
    spawn(async move {
        // Once laid out where it opened: two measures alike.
        let mut last = None;
        let mut settled = None;
        for _ in 0..6 {
            futures_timer::Delay::new(std::time::Duration::from_millis(16)).await;
            let now = measure(&m).await;
            if now.is_some() && now == last {
                settled = now;
                break;
            }
            last = now;
        }
        let layer = viewport.and_then(|v| v.peek().clone());
        let view = match layer {
            Some(l) => measure(&l).await,
            None => None,
        };
        let (Some(r), Some(view)) = (settled.or(last), view) else {
            return;
        };
        // Not yet laid out as itself.
        if r.w <= 0.0 || view.w <= 0.0 {
            return;
        }
        let now = Kept::of(r, view);
        if now != *kept.peek() {
            kept.set(now);
        }
    });
}

/// A menu under the element it opens from, kept in the window ([`Kept`]).
#[component]
pub(crate) fn Popover(ui: ExtensionUi, test: String, children: Element) -> Element {
    let t = crate::window_ui::shell_theme();
    let viewport = try_use_context::<Editor>().map(|e| e.viewport);
    let kept = use_signal(Kept::default);
    let place = kept().css();
    rsx! {
        div {
            role: "menu",
            "data-testid": "{test}",
            "data-kept": kept().mark(),
            style: "position:absolute; {place} z-index:60; min-width:180px; display:flex; flex-direction:column; \
                    padding:4px; border-radius:8px; background:{t.pill}; border:1px solid {t.rule}; box-shadow:{t.shadow_float};",
            onmounted: move |e| keep_in(e.data(), viewport, kept),
            {children}
        }
    }
}

fn dialog_style(t: &architect_shell::ShellTheme, w: u32) -> String {
    format!(
        "width:{w}px; max-height:80%; display:flex; flex-direction:column; gap:8px; padding:12px; \
         border-radius:12px; background:{}; border:1px solid {}; box-shadow:{}; color:{}; font-family:{};",
        t.pill, t.rule, t.shadow_float, t.ink, t.font
    )
}

fn input_style(t: &architect_shell::ShellTheme) -> String {
    format!(
        "height:28px; padding:0 8px; border-radius:6px; border:1px solid {}; background:{}; color:{}; font-size:13px;",
        t.rule, t.sheet, t.ink
    )
}

#[component]
fn Picker(ui: ExtensionUi, picking: Picking) -> Element {
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let mut p = editor.picking;
    let body = match picking {
        Picking::Action(f) => rsx! { ActionPicker { ui, purpose: f } },
        Picking::Widget(f) => rsx! { WidgetPicker { ui, purpose: f } },
    };
    rsx! {
        div {
            style: "position:absolute; left:0; top:0; right:0; bottom:0; pointer-events:auto; z-index:4; display:flex; \
                    align-items:flex-start; justify-content:center; padding-top:72px; background:{t.scrim};",
            onclick: move |_| p.set(None),
            onkeydown: move |e| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    p.set(None);
                }
            },
            div { onclick: move |e| e.stop_propagation(), {body} }
        }
    }
}

/// What picking `id` for `purpose` does.
fn choose_action(editor: Editor, host: &Host, purpose: &For, id: &str) {
    match purpose {
        For::Insert { key, at } => {
            let item = ToolItem {
                action: id.to_owned(),
                ..ToolItem::default()
            };
            let _ = editor.apply(host, key, |d| edit::insert(d, Some(*at), item));
        }
        For::MenuRow { key, at } => {
            let at = *at;
            let _ = editor.apply(host, key, |d| {
                let mut menu = edit::get(d, at).map(|i| i.menu.clone()).unwrap_or_default();
                menu.push(MenuItem {
                    action: id.to_owned(),
                    ..MenuItem::default()
                });
                edit::set(d, at, &json!({ "menu": menu }))
            });
        }
        For::RightClick { key, at } => {
            let _ = editor.apply(host, key, |d| {
                edit::set(d, *at, &json!({ "right_click": id }))
            });
        }
    }
}

/// The actions to pick from: the palette's, else the host's list.
fn commands(host: &Host) -> Vec<Command> {
    if let Some(c) = try_consume_context::<architect_commands::Commands>() {
        let all = c.all();
        if !all.is_empty() {
            return all;
        }
    }
    host.list()
        .infos()
        .into_iter()
        .map(|i| {
            Command::new(i.id, i.display_name)
                .group(i.group)
                .detail(i.description)
        })
        .collect()
}

/// An action, found by the palette's own search.
#[component]
fn ActionPicker(ui: ExtensionUi, purpose: For) -> Element {
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let mut query = use_signal(String::new);
    let host = ui.host();
    let all = commands(&host);
    let found = rank(&all, &query(), &Recents::default())
        .into_iter()
        .filter(|r| r.item.enabled)
        .take(40)
        .map(|r| r.item);
    let input = input_style(&t);
    rsx! {
        div { role: "dialog", "aria-label": "Pick an action", "data-testid": "toolbar-action-picker",
            style: "{dialog_style(&t, 420)}",
            span { style: "font-size:13px; font-weight:600;", "Pick an action" }
            input {
                "data-testid": "toolbar-action-search",
                style: "{input}",
                placeholder: "Search the Action List",
                value: "{query}",
                onmounted: move |e| {
                    let m = e.data();
                    spawn(async move {
                        let _ = m.set_focus(true).await;
                    });
                },
                oninput: move |e| query.set(e.value()),
            }
            div { style: "display:flex; flex-direction:column; gap:1px; overflow-y:auto; max-height:420px;",
                for c in found {
                    {
                        let (id, purpose, host) = (c.id.clone(), purpose.clone(), host.clone());
                        let mut picking = editor.picking;
                        rsx! {
                            button { key: "{c.id}",
                                "data-pick": "{c.id}",
                                style: "display:flex; flex-direction:column; align-items:flex-start; padding:4px 8px; border:none; \
                                        border-radius:6px; background:transparent; color:{t.ink}; cursor:pointer; text-align:left;",
                                onclick: move |_| {
                                    picking.set(None);
                                    choose_action(editor, &host, &purpose, &id);
                                },
                                span { style: "font-size:13px;", "{c.label}" }
                                span { style: "font-size:11px; color:{t.ink_3};", "{c.group} · {c.id}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Every extension's views, by extension: the widget picker's categories.
fn widgets(host: &Host) -> Vec<(String, String, Vec<(String, String)>)> {
    let mut out: Vec<_> = host
        .loaded()
        .into_iter()
        .filter(|l| !l.manifest.views.is_empty())
        .map(|l| {
            let views = l
                .manifest
                .views
                .iter()
                .map(|v| (v.id.clone(), v.title.clone()))
                .collect();
            (l.manifest.id.clone(), l.manifest.name, views)
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// A widget: an extension's view, by its extension, previewed live at a
/// widget's size.
#[component]
fn WidgetPicker(ui: ExtensionUi, purpose: For) -> Element {
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let host = ui.host();
    let all = widgets(&host);
    let mut category = use_signal(|| all.first().map(|c| c.0.clone()));
    let mut chosen = use_signal(|| None::<String>);
    let current = category();
    let views = all
        .iter()
        .find(|c| Some(&c.0) == current.as_ref())
        .map(|c| c.2.clone())
        .unwrap_or_default();
    let picked = chosen();
    let tab = |on: bool| {
        format!(
            "padding:0 10px; height:26px; border:none; border-radius:6px; font-size:12px; cursor:pointer; text-align:left; \
             background:{}; color:{};",
            if on {
                t.accent_soft.as_ref()
            } else {
                "transparent"
            },
            if on {
                t.accent.as_ref()
            } else {
                t.ink.as_ref()
            }
        )
    };
    let insert_host = host;
    let mut picking = editor.picking;
    rsx! {
        div { role: "dialog", "aria-label": "Pick a widget", "data-testid": "toolbar-widget-picker",
            style: "{dialog_style(&t, 560)}",
            span { style: "font-size:13px; font-weight:600;", "Pick a widget" }
            div { style: "display:flex; gap:10px; min-height:220px;",
                div { role: "tablist", style: "flex:0 0 150px; display:flex; flex-direction:column; gap:2px;",
                    for (id, name, _) in all {
                        button { key: "{id}", role: "tab", "data-category": "{id}",
                            "aria-selected": "{current.as_deref() == Some(id.as_str())}",
                            style: "{tab(current.as_deref() == Some(id.as_str()))}",
                            onclick: move |_| {
                                category.set(Some(id.clone()));
                                chosen.set(None);
                            },
                            "{name}"
                        }
                    }
                }
                div { style: "flex:1 1 0; display:flex; flex-direction:column; gap:2px;",
                    for (view, title) in views {
                        {
                            let ext = current.clone().unwrap_or_default();
                            let name = format!("{ext}/{view}");
                            let on = picked.as_deref() == Some(name.as_str());
                            rsx! {
                                button { key: "{name}", "data-widget-pick": "{name}",
                                    style: "{tab(on)}",
                                    onclick: move |_| chosen.set(Some(name.clone())),
                                    "{title}"
                                }
                            }
                        }
                    }
                }
            }
            if let Some(name) = picked {
                {
                    let (extension, view) = name.split_once('/').map(|(e, v)| (e.to_owned(), v.to_owned())).unwrap_or_default();
                    let purpose = purpose;
                    let host = insert_host;
                    rsx! {
                        div { style: "display:flex; align-items:center; gap:10px; padding-top:6px; border-top:1px solid {t.rule};",
                            span { style: "font-size:11px; color:{t.ink_3};", "Preview" }
                            div { "data-testid": "toolbar-widget-preview",
                                style: "flex:0 0 auto; width:160px; height:32px; display:flex; overflow:hidden; border-radius:6px; \
                                        border:1px dashed {t.rule};",
                                ExtensionView { ui, extension, view, fill: true }
                            }
                            span { style: "flex:1 1 0;" }
                            button { "data-testid": "toolbar-widget-insert",
                                style: "border:none; border-radius:6px; padding:0 12px; height:28px; font-size:12px; cursor:pointer; \
                                        background:{t.accent}; color:{t.accent_ink};",
                                onclick: move |_| {
                                    picking.set(None);
                                    if let For::Insert { key, at } = &purpose {
                                        let item = ToolItem {
                                            kind: ToolKind::Widget,
                                            view: name.clone(),
                                            width: Some(160),
                                            ..ToolItem::default()
                                        };
                                        let _ = editor.apply(&host, key, |d| edit::insert(d, Some(*at), item));
                                    }
                                },
                                "Insert"
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The settings of a toolbar (`at` none) or of one of its items.
#[component]
fn SettingsSheet(ui: ExtensionUi, toolbar: String, at: Option<String>) -> Element {
    // The toolbars move under it: read again with every change.
    let (_, _) = (ui.revision(), ui.ticks());
    let t = crate::window_ui::shell_theme();
    let host = ui.host();
    let Some(bar) = host.toolbars().get(host.contributions(), &toolbar) else {
        return rsx! {};
    };
    let close_host = host.clone();
    let place = at.as_deref().and_then(|a| At::parse(a).ok());
    let item = place.and_then(|p| edit::get(&bar.decl, p).cloned());
    let title = match (&item, place) {
        (Some(i), Some(_)) => item_name(&host, i),
        _ => bar.decl.title,
    };
    let what = if item.is_some() { "Item" } else { "Toolbar" };
    rsx! {
        div {
            role: "dialog",
            "aria-label": "{what} settings",
            "data-testid": "toolbar-settings",
            "data-settings-of": "{toolbar}#{at.clone().unwrap_or_default()}",
            style: "position:absolute; top:56px; left:56px; bottom:12px; width:300px; pointer-events:auto; z-index:3; \
                    display:flex; flex-direction:column; gap:10px; padding:12px; overflow-y:auto; border-radius:12px; \
                    background:{t.pill}; border:1px solid {t.rule}; box-shadow:{t.shadow_float}; color:{t.ink}; \
                    font-family:{t.font}; font-size:12px;",
            div { style: "display:flex; align-items:center; gap:6px;",
                span { style: "flex:1 1 0; font-size:13px; font-weight:600;", "{what}: {title}" }
                button {
                    "aria-label": "Close the settings",
                    style: "border:none; background:transparent; color:{t.ink_3}; font-size:14px; cursor:pointer;",
                    onclick: move |_| close_host.toolbars().select(None),
                    "✕"
                }
            }
            match (item, place) {
                (Some(item), Some(at)) => rsx! { ItemSettings { ui, toolbar: toolbar.clone(), at: at.to_string(), item } },
                _ => rsx! { ToolbarSettings { ui, toolbar: toolbar.clone() } },
            }
        }
    }
}

/// A labelled row of the settings.
#[component]
fn Field(label: String, children: Element) -> Element {
    let t = crate::window_ui::shell_theme();
    rsx! {
        div { style: "display:flex; flex-direction:column; gap:4px;",
            span { style: "font-size:11px; color:{t.ink_3};", "{label}" }
            {children}
        }
    }
}

/// Choices side by side, the one on lit.
#[component]
fn Choices(
    choices: Vec<(String, String)>,
    on: String,
    test: String,
    pick: EventHandler<String>,
) -> Element {
    let t = crate::window_ui::shell_theme();
    rsx! {
        div { role: "radiogroup", "data-testid": "{test}", style: "display:flex; flex-wrap:wrap; gap:4px;",
            for (value, label) in choices {
                {
                    let lit = value == on;
                    let v = value.clone();
                    let fill = if lit { t.accent_soft.to_string() } else { "transparent".to_owned() };
                    let ink = if lit { t.accent.to_string() } else { t.ink.to_string() };
                    rsx! {
                        button { key: "{value}", role: "radio", "aria-checked": "{lit}", "data-value": "{value}",
                            style: "padding:0 8px; height:24px; border-radius:6px; font-size:11px; cursor:pointer; \
                                    border:1px solid {t.rule}; background:{fill}; color:{ink};",
                            onclick: move |_| pick.call(v.clone()),
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}

/// A number, one step down or up at a time, in `range`; `-` unset.
#[component]
fn Stepper(
    value: Option<u32>,
    fallback: u32,
    low: u32,
    high: u32,
    step: u32,
    test: String,
    set: EventHandler<u32>,
) -> Element {
    let t = crate::window_ui::shell_theme();
    let now = value.unwrap_or(fallback);
    let b = format!(
        "width:24px; height:24px; border-radius:6px; border:1px solid {}; background:transparent; color:{}; cursor:pointer;",
        t.rule, t.ink
    );
    rsx! {
        div { "data-testid": "{test}", style: "display:flex; align-items:center; gap:6px;",
            button { style: "{b}", "data-step": "down", onclick: move |_| set.call(now.saturating_sub(step).max(low)), "−" }
            span { style: "min-width:28px; text-align:center;", "{now}" }
            button { style: "{b}", "data-step": "up", onclick: move |_| set.call(now.saturating_add(step).min(high)), "+" }
        }
    }
}

/// A colour: a theme token's swatch, or none (the theme's own).
#[component]
fn ColorRow(
    label: String,
    value: String,
    test: String,
    set: EventHandler<Option<String>>,
) -> Element {
    let t = crate::window_ui::shell_theme();
    rsx! {
        Field { label,
            div { "data-testid": "{test}", style: "display:flex; flex-wrap:wrap; gap:4px; align-items:center;",
                button {
                    "data-token": "",
                    title: "The theme's own",
                    "aria-pressed": "{value.is_empty()}",
                    style: "width:20px; height:20px; border-radius:5px; border:1px dashed {t.rule}; background:transparent; \
                            color:{t.ink_3}; font-size:10px; cursor:pointer;",
                    onclick: move |_| set.call(None),
                    "∅"
                }
                for token in COLOR_TOKENS {
                    {
                        let lit = value == token;
                        let ring = if lit { format!("box-shadow:0 0 0 2px {};", t.accent) } else { String::new() };
                        rsx! {
                            button { key: "{token}",
                                "data-token": "{token}",
                                title: "{token}",
                                "aria-pressed": "{lit}",
                                style: "width:20px; height:20px; border-radius:5px; border:1px solid {t.rule}; \
                                        background:var(--{token}); cursor:pointer; {ring}",
                                onclick: move |_| set.call(Some(token.to_owned())),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One item's settings: what it says and shows, its colours, its
/// right-click, a dropdown's rows, a widget's size.
#[allow(clippy::too_many_lines)]
#[component]
fn ItemSettings(ui: ExtensionUi, toolbar: String, at: String, item: ToolItem) -> Element {
    // The toolbars move under it: read again with every change.
    let (_, _) = (ui.revision(), ui.ticks());
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let Ok(place) = At::parse(&at) else {
        return rsx! {};
    };
    let host = ui.host();
    let input = input_style(&t);
    // Sets `patch` on the item.
    let set = {
        let (host, key) = (host.clone(), toolbar.clone());
        move |patch: Value| {
            let _ = editor.apply(&host, &key, |d| edit::set(d, place, &patch));
        }
    };
    let button_like = item.kind == ToolKind::Button;
    let dropdown = button_like && item.action.is_empty();
    let mut image_draft = use_signal(|| item.image.clone());
    let colors = item.colors.clone();
    let (s1, s2, s3, s4, s5, s6, s7, s8, s9) = (
        set.clone(),
        set.clone(),
        set.clone(),
        set.clone(),
        set.clone(),
        set.clone(),
        set.clone(),
        set.clone(),
        set.clone(),
    );
    let colour = move |field: &'static str, s: Box<dyn Fn(Value)>| {
        move |v: Option<String>| {
            s(json!({ "colors": { field: v } }));
        }
    };
    let right = if item.right_click.is_empty() {
        "none".to_owned()
    } else if RIGHT_CLICKS.contains(&item.right_click.as_str()) {
        item.right_click.clone()
    } else {
        "action".to_owned()
    };
    let mut picking = editor.picking;
    let (rc_key, menu_key) = (toolbar.clone(), toolbar.clone());
    let remove_host = host;
    let remove_key = toolbar;
    rsx! {
        if button_like || item.kind == ToolKind::Group {
            Field { label: "Label",
                input { "data-testid": "toolbar-item-label", style: "{input}", value: "{item.label}",
                    placeholder: "Its action's name",
                    oninput: move |e| s1(json!({ "label": e.value() })),
                }
            }
        }
        if button_like {
            Field { label: "Icon (a Lucide name)",
                input { "data-testid": "toolbar-item-icon", style: "{input}", value: "{item.icon}", placeholder: "timer",
                    oninput: move |e| s2(json!({ "icon": e.value() })),
                }
            }
            Field { label: "Image file (in place of the icon)",
                // Set once it names an image file (or nothing): what is typed
                // on the way there is kept here.
                input { "data-testid": "toolbar-item-image", style: "{input}", value: "{image_draft}", placeholder: "icons/star.svg",
                    oninput: move |e| {
                        let v = e.value();
                        image_draft.set(v.clone());
                        if v.is_empty() {
                            s3(json!({ "image": null }));
                        } else if crate::toolbar::image_type(&v).is_some() {
                            s3(json!({ "image": v }));
                        }
                    },
                }
            }
            Field { label: "Its label",
                Choices {
                    test: "toolbar-item-hide-label",
                    choices: vec![("shown".to_owned(), "Shown".to_owned()), ("hidden".to_owned(), "Icon only".to_owned())],
                    on: if item.hide_label { "hidden".to_owned() } else { "shown".to_owned() },
                    pick: move |v: String| s4(json!({ "hide_label": v == "hidden" })),
                }
            }
            ColorRow { label: "Fill", value: colors.fill.clone(), test: "toolbar-item-fill", set: colour("fill", Box::new(s5)) }
            ColorRow { label: "Ink", value: colors.ink.clone(), test: "toolbar-item-ink", set: colour("ink", Box::new(s6)) }
            ColorRow { label: "Fill while on", value: colors.on_fill, test: "toolbar-item-on-fill", set: colour("on_fill", Box::new(s7)) }
            Field { label: "Right-click",
                Choices {
                    test: "toolbar-item-right-click",
                    choices: vec![
                        ("none".to_owned(), "Nothing".to_owned()),
                        ("menu".to_owned(), "Its menu".to_owned()),
                        ("arm".to_owned(), "Arm".to_owned()),
                        ("action".to_owned(), "An action…".to_owned()),
                    ],
                    on: right,
                    pick: move |v: String| match v.as_str() {
                        "action" => picking.set(Some(Picking::Action(For::RightClick { key: rc_key.clone(), at: place }))),
                        "none" => s8(json!({ "right_click": null })),
                        other => s8(json!({ "right_click": other })),
                    },
                }
            }
            if !item.right_click.is_empty() && !RIGHT_CLICKS.contains(&item.right_click.as_str()) {
                span { style: "font-size:11px; color:{t.ink_3};", "Runs {item.right_click}" }
            }
        }
        if dropdown || !item.menu.is_empty() {
            MenuRows { ui, toolbar: menu_key, at: at, rows: item.menu.clone() }
        }
        if item.kind == ToolKind::Widget {
            Field { label: "Width",
                Stepper { test: "toolbar-item-width", value: item.width, fallback: 120, low: 24, high: 480, step: 20,
                    set: move |w: u32| s9(json!({ "width": w })),
                }
            }
            span { style: "font-size:11px; color:{t.ink_3};", "Draws {item.view}" }
        }
        if place.inner.is_none() {
            Field { label: "Row",
                Choices {
                    test: "toolbar-item-newline",
                    choices: vec![("same".to_owned(), "Same row".to_owned()), ("new".to_owned(), "Starts a new row".to_owned())],
                    on: if item.newline { "new".to_owned() } else { "same".to_owned() },
                    pick: move |v: String| set(json!({ "newline": v == "new" })),
                }
            }
        }
        button {
            "data-testid": "toolbar-item-remove",
            style: "margin-top:6px; height:28px; border-radius:6px; border:1px solid {t.danger}; background:transparent; \
                    color:{t.danger}; font-size:12px; cursor:pointer;",
            onclick: move |_| {
                if editor.apply(&remove_host, &remove_key, |d| edit::remove(d, place)).is_some() {
                    remove_host.toolbars().select(None);
                }
            },
            "Remove"
        }
    }
}

/// A dropdown's rows: each its label, up and out; an action, a heading or
/// a separator added at the end.
#[component]
fn MenuRows(ui: ExtensionUi, toolbar: String, at: String, rows: Vec<MenuItem>) -> Element {
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let Ok(place) = At::parse(&at) else {
        return rsx! {};
    };
    let host = ui.host();
    let put = {
        let (host, key) = (host.clone(), toolbar.clone());
        Rc::new(move |rows: Vec<MenuItem>| {
            let _ = editor.apply(&host, &key, |d| {
                edit::set(d, place, &json!({ "menu": rows }))
            });
        })
    };
    let small = format!(
        "height:22px; padding:0 6px; border-radius:5px; border:1px solid {}; background:transparent; color:{}; \
         font-size:11px; cursor:pointer;",
        t.rule, t.ink
    );
    let mut picking = editor.picking;
    let (p_heading, p_sep) = (Rc::clone(&put), Rc::clone(&put));
    let (r_heading, r_sep) = (rows.clone(), rows.clone());
    rsx! {
        Field { label: "Dropdown rows",
            div { "data-testid": "toolbar-menu-rows", style: "display:flex; flex-direction:column; gap:3px;",
                for (n, row) in rows.iter().cloned().enumerate() {
                    {
                        let name = match row.kind {
                            MenuKind::Separator => "— separator —".to_owned(),
                            MenuKind::Submenu => format!("{} ›", row.label),
                            MenuKind::Heading => row.label.clone(),
                            MenuKind::Action if !row.label.is_empty() => row.label.clone(),
                            MenuKind::Action => host.list().info(&row.action).map_or_else(|| row.action.clone(), |i| i.display_name),
                        };
                        let weight = if row.kind == MenuKind::Heading { "600" } else { "400" };
                        let (up, out) = (Rc::clone(&put), Rc::clone(&put));
                        let (ru, ro) = (rows.clone(), rows.clone());
                        rsx! {
                            div { key: "{n}", "data-menu-row": "{n}", style: "display:flex; align-items:center; gap:4px;",
                                span { style: "flex:1 1 0; font-weight:{weight};", "{name}" }
                                if n > 0 {
                                    button { style: "{small}", title: "Up",
                                        onclick: move |_| {
                                            let mut r = ru.clone();
                                            r.swap(n, n - 1);
                                            up(r);
                                        },
                                        "↑"
                                    }
                                }
                                button { style: "{small}", title: "Remove",
                                    onclick: move |_| {
                                        let mut r = ro.clone();
                                        r.remove(n);
                                        out(r);
                                    },
                                    "✕"
                                }
                            }
                        }
                    }
                }
                div { style: "display:flex; gap:4px; flex-wrap:wrap;",
                    button { style: "{small}", "data-testid": "toolbar-menu-add-action",
                        onclick: move |_| picking.set(Some(Picking::Action(For::MenuRow { key: toolbar.clone(), at: place }))),
                        "+ Action…"
                    }
                    button { style: "{small}",
                        onclick: move |_| {
                            let mut r = r_heading.clone();
                            r.push(MenuItem { kind: MenuKind::Heading, label: "Heading".to_owned(), ..MenuItem::default() });
                            p_heading(r);
                        },
                        "+ Heading"
                    }
                    button { style: "{small}",
                        onclick: move |_| {
                            let mut r = r_sep.clone();
                            r.push(MenuItem { kind: MenuKind::Separator, ..MenuItem::default() });
                            p_sep(r);
                        },
                        "+ Separator"
                    }
                }
            }
        }
    }
}

/// A toolbar's own settings: its title and place, sizes, spacing,
/// rounding, labels and colour preset; which toolbar its place shows;
/// duplicate, delete.
#[allow(clippy::too_many_lines)]
#[component]
fn ToolbarSettings(ui: ExtensionUi, toolbar: String) -> Element {
    // The toolbars move under it: read again with every change.
    let (_, _) = (ui.revision(), ui.ticks());
    let t = crate::window_ui::shell_theme();
    let Some(editor) = try_use_context::<Editor>() else {
        return rsx! {};
    };
    let host = ui.host();
    let found = host.toolbars().get(host.contributions(), &toolbar);
    let mut when_draft = use_signal(|| {
        found
            .as_ref()
            .map(|b| b.decl.when.clone())
            .unwrap_or_default()
    });
    let Some(bar) = found else {
        return rsx! {};
    };
    let input = input_style(&t);
    let configure = {
        let (host, key) = (host.clone(), toolbar.clone());
        Rc::new(move |patch: Value| {
            let _ = editor.apply(&host, &key, |d| edit::configure(d, &patch));
        })
    };
    let style = bar.decl.style.clone();
    let style_patch = {
        let c = Rc::clone(&configure);
        let style = style.clone();
        move |f: &dyn Fn(&mut crate::toolbar::ToolbarStyle)| {
            let mut s = style.clone();
            f(&mut s);
            c(json!({ "style": s }));
        }
    };
    let (sp1, sp2, sp3, sp4) = (
        style_patch.clone(),
        style_patch.clone(),
        style_patch.clone(),
        style_patch,
    );
    let (c1, c2, c3, c4) = (
        Rc::clone(&configure),
        Rc::clone(&configure),
        Rc::clone(&configure),
        Rc::clone(&configure),
    );
    let others: Vec<(String, String)> = host
        .toolbars()
        .all(host.contributions())
        .into_iter()
        .filter(|o| o.key != toolbar)
        .map(|o| (o.key, o.decl.title))
        .collect();
    let here = bar.decl.at.clone();
    let shown_here = here != "hidden";
    let (dup_host, del_host, show_host) = (host.clone(), host.clone(), host);
    let (dup_key, del_key) = (toolbar.clone(), toolbar);
    let mut problem = editor.problem;
    let deletes = if bar.source == crate::toolbars::Source::Extension {
        None
    } else if bar.source == crate::toolbars::Source::Override {
        Some("Reset to the extension's")
    } else {
        Some("Delete")
    };
    let small = format!(
        "height:26px; padding:0 10px; border-radius:6px; border:1px solid {}; background:transparent; color:{}; \
         font-size:12px; cursor:pointer;",
        t.rule, t.ink
    );
    rsx! {
        Field { label: "Title",
            input { "data-testid": "toolbar-title", style: "{input}", value: "{bar.decl.title}",
                oninput: move |e| {
                    let v = e.value();
                    if !v.trim().is_empty() {
                        c1(json!({ "title": v }));
                    }
                },
            }
        }
        Field { label: "Where",
            Choices {
                test: "toolbar-where",
                choices: PLACES.iter().map(|(v, l)| ((*v).to_owned(), (*l).to_owned())).collect::<Vec<_>>(),
                on: here.clone(),
                pick: move |v: String| c2(json!({ "where": v })),
            }
        }
        Field { label: "Only for (a context: view:note, editor:markdown; empty, every one)",
            input { "data-testid": "toolbar-when", style: "{input}", value: "{when_draft}", placeholder: "every context",
                oninput: move |e| {
                    let v = e.value();
                    when_draft.set(v.clone());
                    c4(json!({ "when": if v.trim().is_empty() { Value::Null } else { Value::String(v.trim().to_owned()) } }));
                },
            }
        }
        Field { label: "Button size",
            Stepper { test: "toolbar-size", value: style.size, fallback: 28, low: *SIZES.start(), high: *SIZES.end(), step: 4,
                set: move |v: u32| sp1(&|s| s.size = Some(v)) }
        }
        Field { label: "Spacing",
            Stepper { test: "toolbar-spacing", value: style.spacing, fallback: 4, low: 0, high: 24, step: 2,
                set: move |v: u32| sp2(&|s| s.spacing = Some(v)) }
        }
        Field { label: "Rounding",
            Stepper { test: "toolbar-rounding", value: style.rounding, fallback: 6, low: 0, high: 20, step: 2,
                set: move |v: u32| sp3(&|s| s.rounding = Some(v)) }
        }
        Field { label: "Labels",
            Choices {
                test: "toolbar-labels",
                choices: vec![("shown".to_owned(), "Shown".to_owned()), ("hidden".to_owned(), "Icons only".to_owned())],
                on: if bar.decl.hide_labels { "hidden".to_owned() } else { "shown".to_owned() },
                pick: move |v: String| c3(json!({ "hide_labels": v == "hidden" })),
            }
        }
        Field { label: "Colours",
            Choices {
                test: "toolbar-preset",
                choices: PRESETS.iter().map(|p| ((*p).to_owned(), (*p).to_owned())).collect::<Vec<_>>(),
                on: if style.preset.is_empty() { "theme".to_owned() } else { style.preset },
                pick: move |v: String| sp4(&|s| s.preset = if v == "theme" { String::new() } else { v.clone() }),
            }
        }
        if shown_here && !others.is_empty() {
            Field { label: "Show another here instead",
                div { "data-testid": "toolbar-switch", style: "display:flex; flex-direction:column; gap:2px;",
                    for (key, title) in others {
                        {
                            let (host, at, k) = (show_host.clone(), here.clone(), key.clone());
                            rsx! {
                                button { key: "{key}", "data-switch-to": "{key}", style: "{small} text-align:left;",
                                    onclick: move |_| {
                                        let bars = host.toolbars();
                                        match bars.show(host.contributions(), &k, &at) {
                                            Ok(_) => {
                                                write_later(&host, bars.last_edit());
                                                bars.select(Some((k.clone(), None)));
                                            }
                                            Err(why) => problem.set(Some(why)),
                                        }
                                    },
                                    "{title}"
                                }
                            }
                        }
                    }
                }
            }
        }
        div { style: "display:flex; gap:6px; margin-top:6px;",
            button { "data-testid": "toolbar-duplicate", style: "{small}",
                onclick: move |_| {
                    let bars = dup_host.toolbars();
                    match bars.duplicate(dup_host.contributions(), &dup_key, None, None) {
                        Ok(key) => bars.select(Some((key, None))),
                        Err(why) => problem.set(Some(why)),
                    }
                },
                "Duplicate"
            }
            if let Some(label) = deletes {
                button { "data-testid": "toolbar-delete",
                    style: "{small} border-color:{t.danger}; color:{t.danger};",
                    onclick: move |_| {
                        let bars = del_host.toolbars();
                        bars.select(None);
                        if let Err(why) = bars.reset(&del_key) {
                            problem.set(Some(why));
                        }
                    },
                    "{label}"
                }
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn placed(index: usize, inner: Option<usize>, x: f32, w: f32, group: bool) -> Placed {
        Placed {
            at: At { index, inner },
            rect: Rect::new(x, 10.0, w, 28.0),
            group,
        }
    }

    /// A toolbar at x 0..400: A (0..40), a group (50..150) of two (55..95,
    /// 100..140), B (160..200).
    fn strips() -> Vec<Strip> {
        vec![
            Strip {
                key: "user/a".into(),
                rect: Rect::new(0.0, 0.0, 400.0, 48.0),
                items: vec![
                    placed(0, None, 0.0, 40.0, false),
                    placed(1, None, 50.0, 100.0, true),
                    placed(1, Some(0), 55.0, 40.0, false),
                    placed(1, Some(1), 100.0, 40.0, false),
                    placed(2, None, 160.0, 40.0, false),
                ],
            },
            Strip {
                key: "user/empty".into(),
                rect: Rect::new(0.0, 100.0, 400.0, 48.0),
                items: Vec::new(),
            },
        ]
    }

    #[test]
    fn a_menu_past_the_windows_edge_is_kept_in_it() {
        let view = Rect::new(0.0, 0.0, 1440.0, 860.0);
        assert_eq!(
            Kept::of(Rect::new(100.0, 80.0, 180.0, 120.0), view),
            Kept::default()
        );
        // Past the right edge: left by as much.
        let k = Kept::of(Rect::new(1350.0, 80.0, 180.0, 120.0), view);
        assert!((k.shift - 94.0).abs() < 0.01 && !k.up, "{k:?}");
        assert_eq!(k.css(), "top:calc(100% + 4px); left:-94px;");
        assert_eq!(k.mark().as_deref(), Some("left 94"));
        // Past the bottom with room above: upward.
        assert!(Kept::of(Rect::new(100.0, 800.0, 180.0, 120.0), view).up);
    }

    #[test]
    fn a_drop_lands_between_items_into_groups_and_other_toolbars() {
        let s = strips();
        let at =
            |x: f32, y: f32, g: bool| drop_slot(&s, x, y, g).map(|d| (d.key, d.at.to_string()));
        assert_eq!(at(10.0, 20.0, false), Some(("user/a".into(), "0".into())));
        assert_eq!(at(30.0, 20.0, false), Some(("user/a".into(), "1".into())));
        assert_eq!(at(5.0, 20.0, false), Some(("user/a".into(), "0".into())));
        // Over the group: into it, between its buttons.
        assert_eq!(at(70.0, 20.0, false), Some(("user/a".into(), "1.0".into())));
        assert_eq!(
            at(130.0, 20.0, false),
            Some(("user/a".into(), "1.2".into()))
        );
        // A group itself goes beside it.
        assert_eq!(at(130.0, 20.0, true), Some(("user/a".into(), "2".into())));
        assert_eq!(at(390.0, 20.0, false), Some(("user/a".into(), "3".into())));
        // An empty toolbar: its start.
        assert_eq!(
            at(200.0, 120.0, false),
            Some(("user/empty".into(), "0".into()))
        );
        // Off every toolbar: nowhere.
        assert_eq!(at(200.0, 80.0, false), None);
    }
}
