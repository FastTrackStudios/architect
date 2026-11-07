//! What loaded extensions add, in the app's window (feature `ui`):
//!
//! the shell's chrome (tabs, status items, a phone's bar buttons, the slots
//! they fill), a tab's and a file's context menus, the menu bar, their
//! toolbars (`crate::toolbar_ui`), their views as workspace views, where
//! those open (tab, split, dock, float, zoomed, popup, overlay), and a
//! link's Ctrl- or Cmd-click
//! (`docs/content/architecture/extensions.md`, "Contributions").
//!
//! ```ignore
//! let ui = use_extension_ui(host.clone(), app_list.clone());
//! chrome.set(ui.chrome()); // over whatever a layout file says
//! rsx! {
//!     AppShell { tabs, on_tab: move |id| if !ui.is_tab(id) { … } }
//!     ExtensionOverlays { ui }
//! }
//! ```
//!
//! An extension's views are mounted by [`ExtensionView`]: the component
//! of its linked-in copy, or its process's remote view (feature
//! `remote`), or — once it is unloaded — a placeholder saying so (a view
//! whose manifest says `on_unload = "close"` has its panes closed).
//! An app that runs pages provides [`ViewPages`]: a view with a page file
//! (its manifest's `page`, or an override in the app's folder) is drawn
//! from that file instead (`architect-pages`).

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::{ActionList, Invocation, InvocationSource};
use architect_commands::Live;
use architect_shell::{
    BarAction, Chrome, Filler, Slot, StatusItem, StatusSide, Tab, intern, named_icon,
};
use architect_workspace::{ContextEntries, ContextEntry, Side, Target, View};
use dioxus::prelude::*;
use futures::StreamExt as _;
use futures::channel::mpsc;
use serde_json::{Value, json};

use crate::contrib::{self, Contributions};
use crate::host::{Host, Opening, ViewOpen, now_ms};
use crate::manifest::{Home, OnUnload, ViewDecl, ViewPlacement};

/// The contributions' revision as a signal: it moves when they change,
/// and when a piece of runtime state lapses.
#[must_use]
pub fn use_contributions(contributions: &Contributions) -> Signal<u64> {
    let mut revision = use_signal(|| contributions.revision());
    let reg = contributions.clone();
    use_future(move || {
        let reg = reg.clone();
        async move {
            let (tx, mut rx) = mpsc::unbounded::<u64>();
            let _held = reg.subscribe(move |r| {
                let _ = tx.unbounded_send(r);
            });
            loop {
                // A lapse is a change too: wake for the next one.
                let lapse = reg
                    .next_expiry()
                    .map(|at| at.saturating_sub(now_ms()).max(50));
                let next = match lapse {
                    Some(ms) => {
                        let wait = futures_timer::Delay::new(std::time::Duration::from_millis(ms));
                        futures::select! {
                            r = rx.next() => r,
                            () = futures::FutureExt::fuse(wait) => Some(reg.revision()),
                        }
                    }
                    None => rx.next().await,
                };
                if next.is_none() {
                    return;
                }
                // Several at once are one redraw.
                while rx.try_recv().is_ok() {}
                let n = revision.peek().wrapping_add(1);
                revision.set(n);
            }
        }
    });
    revision
}

/// A view an extension opened over the window (`popup`, `overlay`).
#[derive(Clone, Debug, PartialEq)]
struct Layer {
    open: ViewOpen,
}

/// What loaded extensions add, for the window. Cheap to copy.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExtensionUi {
    host: Signal<Host>,
    revision: Signal<u64>,
    layers: Signal<Vec<Layer>>,
    /// Every view an extension declared since the app started, by kind:
    /// what an unloaded one's panes show (or close by).
    seen: Signal<BTreeMap<String, (String, ViewDecl)>>,
    /// The extensions' windows open now (`crate::window_ui`).
    windows: Signal<Vec<crate::windows::OpenWindow>>,
    /// Moves when anything happens on the host's Action List (an action
    /// ran, the app's state changed): what a toggle's button re-reads.
    ticks: Signal<u64>,
    /// The toolbars that are panes of their own, by kind, with their
    /// title and place: what the views follow (a person's new toolbar).
    pub(crate) toolbar_panes: Signal<Vec<(String, String, String)>>,
}

impl ExtensionUi {
    /// The host.
    #[must_use]
    pub fn host(&self) -> Host {
        self.host.peek().clone()
    }

    /// The contributions' revision (reading it subscribes).
    #[must_use]
    pub fn revision(&self) -> u64 {
        (self.revision)()
    }

    /// How many times something happened on the host's Action List since
    /// the window opened (reading it subscribes): a toggle may have moved.
    #[must_use]
    pub fn ticks(&self) -> u64 {
        (self.ticks)()
    }

    /// The extensions' windows open now (reading it subscribes).
    #[must_use]
    pub fn windows(&self) -> Vec<crate::windows::OpenWindow> {
        (self.windows)()
    }

    /// The shell's chrome as the extensions have it: their tabs, status
    /// items and bar buttons, and the slots they fill. Put it under a
    /// layout file's (`Chrome::over`).
    #[must_use]
    pub fn chrome(&self) -> Chrome {
        let _ = (self.revision)();
        let host = self.host();
        let reg = host.contributions();
        let commands = try_consume_context::<architect_commands::Commands>();
        let mut chrome = Chrome::default();
        for by in reg.tabs() {
            let t = &by.item;
            chrome.tabs.push(Tab::new(
                intern(&format!("{}/{}", by.owner, t.id)),
                intern(&t.title),
                named_icon(&t.icon),
            ));
        }
        for by in reg.status(now_ms()) {
            let s = by.item;
            let mut item = StatusItem::new(format!("{}/{}", by.owner, s.id), s.text);
            if !s.icon.is_empty() {
                item = item.icon(named_icon(&s.icon));
            }
            if !s.hint.is_empty() {
                item = item.hint(s.hint);
            }
            if s.side == contrib::StatusSide::Left {
                item.side = StatusSide::Left;
            }
            if !s.action.is_empty()
                && let Some(c) = commands
            {
                let action = s.action.clone();
                item = item.on_press(move |()| {
                    let _ = c.run_invoked(&action, None, Invocation::new(InvocationSource::App));
                });
            }
            chrome.status.push(item);
        }
        for by in reg.bar() {
            chrome.bar.push(BarAction::new(
                by.item.action.clone(),
                if by.item.title.is_empty() {
                    by.item.action.clone()
                } else {
                    by.item.title.clone()
                },
                named_icon(&by.item.icon),
            ));
        }
        for (slot, (owner, view)) in reg.slots() {
            let Some(slot) = Slot::named(&slot) else {
                continue;
            };
            let me = *self;
            let kind = format!("{owner}/{view}");
            let (e, v) = (owner.clone(), view.clone());
            chrome.fillers.insert(
                slot,
                Filler::new(format!("ext:{owner}"), kind, move || {
                    rsx! { ExtensionView { ui: me, extension: e.clone(), view: v.clone(), fill: false } }
                }),
            );
        }
        chrome
    }

    /// The extension and tab a contributed tab's id names
    /// (`lab.board/board`), when it is one.
    #[must_use]
    pub fn tab(&self, id: &str) -> Option<(String, contrib::TabDecl)> {
        let host = self.host();
        host.contributions()
            .tabs()
            .into_iter()
            .find(|by| format!("{}/{}", by.owner, by.item.id) == id)
            .map(|by| (by.owner, by.item))
    }

    /// Whether `id` is a contributed tab's.
    #[must_use]
    pub fn is_tab(&self, id: &str) -> bool {
        self.tab(id).is_some()
    }

    /// The menu bar's contributed rows for menu `menu` (`MenuBar::contributed`).
    #[must_use]
    pub fn menu_rows(&self, menu: &str) -> Vec<Live> {
        let host = self.host();
        host.contributions()
            .menus(menu)
            .into_iter()
            .filter_map(|by| {
                let info = host.list().info(&by.item.action)?;
                let mut row = Live::new(info.display_name, by.item.action);
                row.checked = info.checked;
                Some(row)
            })
            .collect()
    }

    /// Every loaded extension's views as workspace views, and a
    /// placeholder for each one seen before that is gone: give them to the
    /// app's [`architect_workspace::Views`].
    #[must_use]
    pub fn views(&self) -> Vec<View> {
        let _ = (self.revision)();
        let _ = (self.toolbar_panes)();
        let me = *self;
        let mut all = self.seen.peek().clone();
        all.extend(declared(&self.host()));
        all.iter()
            .map(|(kind, (extension, decl))| {
                let (e, v) = (extension.clone(), decl.id.clone());
                let mut view = View::new(kind.clone(), decl.title.clone(), move |_| {
                    rsx! { ExtensionView { ui: me, extension: e.clone(), view: v.clone() } }
                })
                .home(home(decl.home))
                .still();
                if !decl.icon.is_empty() {
                    view = view.icon(named_icon(&decl.icon));
                }
                view
            })
            .chain(crate::toolbar_ui::pane_views(me))
            .collect()
    }

    /// Opens `url` with the extension opener that claims it, as a link's
    /// Ctrl- or Cmd-click does: false when none does.
    #[must_use = "false when nothing opens it"]
    pub fn open_link(&self, url: &str) -> bool {
        let host = self.host();
        if host.contributions().open_url(url).is_empty() {
            return false;
        }
        let url = url.to_owned();
        spawn(async move {
            let _ = host
                .open(Opening::Url(url), Invocation::new(InvocationSource::Link))
                .await;
        });
        true
    }

    /// Opens the file at `path` with the extension opener that claims it:
    /// false when none does.
    #[must_use = "false when nothing opens it"]
    pub fn open_path(&self, path: &str) -> bool {
        let host = self.host();
        if host.contributions().open_path(path).is_empty() {
            return false;
        }
        let path = path.to_owned();
        spawn(async move {
            let _ = host
                .open(Opening::Path(path), Invocation::new(InvocationSource::Open))
                .await;
        });
        true
    }

    /// Takes a popup or an overlay away.
    pub fn close_layer(&self, kind: &str) {
        let mut layers = self.layers;
        layers.write().retain(|l| l.open.kind != kind);
    }
}

/// Every loaded extension's views, by workspace kind.
fn declared(host: &Host) -> BTreeMap<String, (String, ViewDecl)> {
    host.loaded()
        .into_iter()
        .flat_map(|l| {
            let id = l.manifest.id.clone();
            // A view the palette slot draws (`[[palette_views]]`) is an
            // overlay of the shell's, not a pane.
            let slotted: Vec<String> = l
                .manifest
                .contributes
                .palette_views
                .iter()
                .map(|p| p.view.clone())
                .collect();
            l.manifest
                .views
                .into_iter()
                .filter(move |v| !slotted.contains(&v.id))
                .map(move |v| (v.workspace_kind(&id), (id.clone(), v)))
        })
        .collect()
}

/// Whether a link press is the one that opens it with an extension: a
/// press with Ctrl or Cmd held.
#[must_use]
pub fn link_press(e: &MouseEvent) -> bool {
    let m = e.modifiers();
    m.ctrl() || m.meta()
}

const fn home(h: Home) -> Target {
    match h {
        Home::Main => Target::Tab,
        Home::Left => Target::Dock(Side::Left),
        Home::Right => Target::Dock(Side::Right),
        Home::Bottom => Target::Dock(Side::Bottom),
    }
}

/// Where `open` goes, as `WORKSPACE_OPEN`'s `at`, next to `active`.
fn at(open: &ViewOpen, near: &Near) -> Value {
    let active = near.document.as_str();
    match (open.home, open.placement) {
        (Home::Left, _) => json!({ "dock": { "side": "left" } }),
        (Home::Right, _) => json!({ "dock": { "side": "right" } }),
        (Home::Bottom, _) => json!({ "dock": { "side": "bottom" } }),
        (_, ViewPlacement::Float) => json!({ "float": { "rect": {
            "x": 120.0, "y": 90.0,
            "w": open.width.unwrap_or(420.0), "h": open.height.unwrap_or(320.0),
        } } }),
        (_, ViewPlacement::Split) if !active.is_empty() => {
            json!({ "split": { "pane": active, "side": "right" } })
        }
        _ if !active.is_empty() => json!({ "beside": { "pane": active } }),
        _ => near.main.map_or_else(
            || json!({ "float": { "rect": { "x": 120.0, "y": 90.0, "w": 480.0, "h": 360.0 } } }),
            |group| json!({ "group": { "group": group } }),
        ),
    }
}

/// Where a view of the main area opens: beside the document on show (the
/// most recent pane in a main area, never a dock's: #442, the Projects page
/// took the navigator's place when the navigator had the focus), else in
/// the main window's first group.
#[derive(Debug, Default)]
struct Near {
    document: String,
    main: Option<u64>,
}

impl Near {
    fn of(layout: &Value) -> Self {
        Self {
            document: layout
                .get("current_document")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            main: layout
                .pointer("/windows/0/groups/0/id")
                .and_then(Value::as_u64),
        }
    }
}

/// Opens `open` in the workspace through the app's list: a pane of its
/// kind (its id the kind, so opening it again brings it forward), zoomed
/// when it says so.
async fn open_in_workspace(list: &ActionList, open: &ViewOpen) -> Result<Value, String> {
    let layout = list
        .run_json_async("WORKSPACE_LAYOUT", b"")
        .await
        .map_err(|e| format!("no workspace to open it in: {e}"))?;
    let layout: Value = serde_json::from_slice(&layout).unwrap_or(Value::Null);
    let near = Near::of(&layout);
    let args = json!({
        "view": open.kind,
        "id": open.kind,
        "state": open.state,
        "title": open.title,
        "at": at(open, &near),
        "focus": true,
    });
    let out = list
        .run_json_async("WORKSPACE_OPEN", args.to_string().as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    if open.placement == ViewPlacement::Zoomed {
        let zoom = json!({ "pane": open.kind, "mode": "on" }).to_string();
        let _ = list.run_json_async("WORKSPACE_ZOOM", zoom.as_bytes()).await;
    }
    Ok(serde_json::from_slice(&out).unwrap_or(Value::Null))
}

/// Puts what `host`'s extensions add into the window: call it in the
/// app's root component, above the shell. `app` is the app's Action List
/// (where `WORKSPACE_*` are): views open through it.
///
/// It provides the workspace's [`ContextEntries`] (an extension's
/// `[[context]]` rows) and sets the host's view opener; read the rest
/// from the [`ExtensionUi`] it gives.
pub fn use_extension_ui(host: Host, app: Arc<ActionList>) -> ExtensionUi {
    let revision = use_contributions(host.contributions());
    let layers = use_signal(Vec::<Layer>::new);
    let seen = use_signal(BTreeMap::new);
    let held = use_signal(|| host.clone());
    let windows = crate::window_ui::use_open_windows(&host);
    let ticks = crate::toolbar_ui::use_ticks(&host);
    let toolbar_panes = use_signal(Vec::new);
    let ui = ExtensionUi {
        host: held,
        revision,
        layers,
        seen,
        windows,
        ticks,
        toolbar_panes,
    };
    // The settings page is the settings extension's window: the app's
    // sections and values to it, `SettingsHandle::show` opening it.
    crate::window_ui::use_settings_window(&host, windows);
    // The settings page draws an extension's section from its view
    // (`[[sections]] view`), a linked one's component as it is.
    use_context_provider(move || {
        architect_settings::SectionViews(Rc::new(move |kind: &str| {
            let (extension, view) = kind.split_once('/')?;
            let host = ui.host();
            if let Some(root) = host.linked_view(extension, view) {
                return Some(rsx! { LinkedRoot { key: "{kind}", root } });
            }
            host.get(extension)?;
            Some(rsx! {
                div { style: "flex: 1; min-height: 480px; display: flex; flex-direction: column;",
                    ExtensionView { ui, extension: extension.to_owned(), view: view.to_owned() }
                }
            })
        }))
    });
    // ⌘O: every extension's quick-open openers, the palette's own.
    use_quick_open(host.clone(), revision);
    // The palette slot's views, the keys extensions bind and their
    // palette sources (#381): ⌘K is the Command Palette extension's.
    use_palette(ui, host.clone(), revision);
    // A tab's icons and badges (`decorate` on a `tab`): by its view kind
    // and by its pane.
    crate::host_ui_view::use_tab_badges(&host);
    // A tab's and a file's context menus: an extension's rows for them,
    // a toggle's ticked when on.
    let reg = host.contributions().clone();
    let list = host.list().clone();
    let bars_host = host.clone();
    use_context_provider(move || {
        ContextEntries::new(move |target, id| {
            // The toolbars' own rows first (`toolbar`, `toolbar-item`).
            let mut rows = crate::toolbar_ui::context_rows(&bars_host, target, id);
            rows.extend(reg.context(target, id).into_iter().filter_map(|by| {
                let info = list.info(&by.item.action)?;
                let label = if by.item.title.is_empty() {
                    info.display_name
                } else {
                    by.item.title.clone()
                };
                Some(ContextEntry {
                    label,
                    command: by.item.action,
                    checked: info.checked,
                })
            }));
            rows
        })
    });
    // The toolbar editor's state, shared by the window's toolbars.
    let _ = crate::toolbar_editor::provide_editor();
    // A pane's header: the toolbars on every pane, and on its view's.
    crate::toolbar_ui::provide_header_tools(ui);
    // A phone's Typing Toolbar: the focused editor's own items and the
    // toolbars of place `typing` for where the focus is.
    crate::typing_ui::provide_typing_tools(ui);
    // A toolbar that is a pane of its own (a dock's, a float, a window)
    // opens where it goes as it appears, and closes as it goes.
    crate::toolbar_ui::use_pane_toolbars(ui, Arc::clone(&app));
    // Its views open in the window: a pane, or a layer over it.
    let close_host = host.clone();
    let opener = use_hook(move || {
        let (tx, rx) = mpsc::unbounded::<ViewOpen>();
        let busy = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let seen_busy = Arc::clone(&busy);
        host.set_view_opener(move |open| {
            if open.placement == ViewPlacement::Popup {
                let mut b = seen_busy
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if b.iter().any(|k| *k != open.kind) {
                    return Err(format!("busy: another popup ({}) is open", b.join(", ")));
                }
                b.push(open.kind.clone());
            }
            let placed = format!("{:?}", open.placement).to_lowercase();
            tx.unbounded_send(open)
                .map_err(|_| "the window is gone".to_owned())?;
            Ok(json!({ "placement": placed }))
        });
        Rc::new(std::cell::RefCell::new(Some((rx, busy))))
    });
    // An extension's EXT_VIEW_CLOSE takes its popup or overlay away.
    let closing = use_hook(move || {
        let (tx, rx) = mpsc::unbounded::<String>();
        close_host.set_view_closer(move |kind| {
            tx.unbounded_send(kind.to_owned())
                .map_err(|_| "the window is gone".to_owned())
        });
        Rc::new(std::cell::RefCell::new(Some(rx)))
    });
    use_future(move || {
        let taken = closing.borrow_mut().take();
        async move {
            let Some(mut rx) = taken else { return };
            let mut layers = layers;
            while let Some(kind) = rx.next().await {
                layers.write().retain(|l| l.open.kind != kind);
            }
        }
    });
    let mut layers_w = layers;
    let closer = Arc::clone(&app);
    use_future(move || {
        let taken = opener.borrow_mut().take();
        let app = Arc::clone(&app);
        async move {
            let Some((mut rx, busy)) = taken else { return };
            while let Some(open) = rx.next().await {
                match open.placement {
                    ViewPlacement::Popup | ViewPlacement::Overlay => {
                        layers_w.write().retain(|l| l.open.kind != open.kind);
                        layers_w.write().push(Layer { open });
                    }
                    _ => {
                        if let Err(why) = open_in_workspace(&app, &open).await {
                            tracing::warn!(ext.id = %open.extension, ext.view = %open.kind, ext.why = %why, "a view did not open");
                        }
                    }
                }
                busy.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .retain(|k| layers_w.peek().iter().any(|l| l.open.kind == *k));
            }
        }
    });
    // Every view seen is remembered (an unloaded one's panes say so); an
    // unloaded extension's views that close by their manifest, close; its
    // layers go.
    let mut seen_w = seen;
    use_effect(move || {
        let _ = revision();
        let host = held.peek().clone();
        let now = declared(&host);
        if now.iter().any(|(k, v)| seen_w.peek().get(k) != Some(v)) {
            seen_w.write().extend(now.clone());
        }
        let gone: Vec<(String, ViewDecl)> = seen_w
            .peek()
            .iter()
            .filter(|(k, _)| !now.contains_key(*k))
            .map(|(k, (_, d))| (k.clone(), d.clone()))
            .collect();
        if gone.is_empty() {
            return;
        }
        layers_w
            .write()
            .retain(|l| !gone.iter().any(|(k, _)| *k == l.open.kind));
        let closing: Vec<String> = gone
            .into_iter()
            .filter(|(_, d)| d.on_unload == OnUnload::Close)
            .map(|(k, _)| k)
            .collect();
        for pane in closing {
            let args = json!({ "panes": [pane] }).to_string();
            let app = Arc::clone(&closer);
            spawn(async move {
                let _ = app.run_json_async("WORKSPACE_CLOSE", args.as_bytes()).await;
            });
        }
    });
    ui
}

/// Keeps ⌘O's openers what the extensions' `[[openers]]` (the ones with a
/// `kind`, not the claims) say, as they load and go.
fn use_quick_open(host: Host, revision: Signal<u64>) {
    let Some(commands) = try_use_context::<architect_commands::Commands>() else {
        return;
    };
    let mine = use_hook(|| Rc::new(std::cell::RefCell::new(Vec::<String>::new())));
    // What an item picked runs, here: ⌘O closes as it opens.
    let opens = use_coroutine(
        move |mut asks: mpsc::UnboundedReceiver<(ActionList, String, String)>| async move {
            while let Some((list, open, args)) = asks.next().await {
                let caller = architect::action::ActionCaller::app()
                    .with_invocation(Invocation::new(InvocationSource::Palette));
                if let Err(e) = list
                    .run_json_async_with(&caller, &open, args.as_bytes())
                    .await
                {
                    tracing::warn!(ext.action = %open, error = %e, "an extension's ⌘O item did not open");
                }
            }
        },
    );
    use_effect(move || {
        let _ = revision();
        let now = host.contributions().quick_open();
        let kinds: Vec<String> = now.iter().map(|o| o.item.kind.clone()).collect();
        for gone in mine.borrow().iter().filter(|k| !kinds.contains(k)) {
            commands.remove_opener(gone);
        }
        for by in now {
            commands.register_opener(quick_opener(
                host.list().clone(),
                &commands,
                opens.tx(),
                &by.item,
            ));
        }
        *mine.borrow_mut() = kinds;
    });
}

/// Keeps the shell's palette slot what the loaded extensions say, as they
/// load and go (#381): which view draws each overlay (`[[palette_views]]`;
/// the Command Palette's, a development copy's in its place), the keys
/// they bind (`[[keys]]`: its ⌘K and ⌘O), and their sources of the
/// palette's rows (`[[palette_sources]]`), ranked by the engine.
fn use_palette(ui: ExtensionUi, host: Host, revision: Signal<u64>) {
    let Some(commands) = try_use_context::<architect_commands::Commands>() else {
        return;
    };
    let mine = use_hook(|| Rc::new(std::cell::RefCell::new(Vec::<String>::new())));
    // What a row picked runs, here: the palette closes as it runs.
    let runs = use_coroutine(
        move |mut asks: mpsc::UnboundedReceiver<(ActionList, String, String)>| async move {
            while let Some((list, run, args)) = asks.next().await {
                let caller = architect::action::ActionCaller::app()
                    .with_invocation(Invocation::new(InvocationSource::Palette));
                if let Err(e) = list
                    .run_json_async_with(&caller, &run, args.as_bytes())
                    .await
                {
                    tracing::warn!(ext.action = %run, error = %e, "an extension's palette row did not run");
                }
            }
        },
    );
    use_effect(move || {
        let _ = revision();
        let contributions = host.contributions();
        let views = contributions
            .palette_views()
            .into_iter()
            .filter_map(|by| {
                let overlay = architect_commands::Overlay::parse(&by.item.overlay)?;
                let (extension, view) = (by.owner.clone(), by.item.view.clone());
                let draw: Rc<dyn Fn() -> Element> =
                    Rc::new(move || palette_view(ui, extension.clone(), view.clone()));
                Some(architect_commands::PaletteView {
                    overlay,
                    owner: by.owner,
                    draw,
                })
            })
            .collect();
        commands.set_palette_views(views);
        commands.set_bound_keys(
            contributions
                .key_bindings()
                .into_iter()
                .map(|by| (by.item.action, by.item.chord))
                .collect(),
        );
        let now = contributions.palette_sources();
        let ids: Vec<String> = now.iter().map(|p| p.item.id.clone()).collect();
        for gone in mine.borrow().iter().filter(|id| !ids.contains(id)) {
            commands.remove_source(gone);
        }
        for by in now {
            let decl = by.item;
            let ranking = if decl.ranking == "commands" {
                architect_commands::Ranking::Commands
            } else {
                architect_commands::Ranking::Open
            };
            let id = decl.id.clone();
            let items = asked(
                host.list().clone(),
                &commands,
                decl.action.clone(),
                move |out| palette_items(&id, out),
            );
            let source =
                architect_commands::FnSource::new(decl.id.clone(), decl.title.clone(), items)
                    .ranking(ranking);
            let (run, by_id) = if decl.run.is_empty() {
                (decl.action.clone(), false)
            } else {
                (decl.run.clone(), true)
            };
            let (list, tx) = (host.list().clone(), runs.tx());
            commands.register_source(source, move |item: &architect_commands::engine::Item| {
                let key = if by_id { "id" } else { "run" };
                let args = json!({ key: item.id }).to_string();
                let _ = tx.unbounded_send((list.clone(), run.clone(), args));
            });
        }
        *mine.borrow_mut() = ids;
    });
}

/// Extension `extension`'s view `view` in the palette slot: a linked one's
/// component as it is (it draws its own scrim and panel, the Command
/// Palette's `Frame`); a process's (a development copy) in a panel where
/// the palette's goes.
fn palette_view(ui: ExtensionUi, extension: String, view: String) -> Element {
    if ui.host().linked_view(&extension, &view).is_some() {
        return rsx! {
            ExtensionView { ui, extension, view, bare: true }
        };
    }
    let commands = try_use_context::<architect_commands::Commands>();
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: 90; background: rgba(0,0,0,0.45); \
                    display: flex; justify-content: center; align-items: flex-start; padding-top: 12vh;",
            onclick: move |_| {
                if let Some(c) = commands {
                    c.close();
                }
            },
            div {
                style: "width: 620px; max-width: 92%; height: 60vh; display: flex; flex-direction: column; \
                        border-radius: 12px; overflow: hidden; background: var(--card); border: 1px solid var(--border); \
                        box-shadow: 0 24px 64px rgba(0,0,0,0.6);",
                onclick: move |e| e.stop_propagation(),
                ExtensionView { ui, extension, view }
            }
        }
    }
}

/// A palette source action's answer (`{items: [{id, label, detail?,
/// group?, shortcut?, recency?}]}`; `title` for `label` too) as the
/// engine's items of source `source`.
fn palette_items(source: &str, out: Option<Vec<u8>>) -> Vec<architect_commands::engine::Item> {
    let answer: Value = out
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let list = answer
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    list.iter()
        .filter_map(|v| {
            let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
            let id = text("id");
            let label = [text("label"), text("title"), id]
                .into_iter()
                .find(|s| !s.is_empty())?;
            (!id.is_empty()).then(|| {
                architect_commands::engine::Item::new(id, label)
                    .source(source)
                    .group(text("group"))
                    .detail(text("detail"))
                    .shortcut(text("shortcut"))
                    .recency(v.get("recency").and_then(Value::as_u64).unwrap_or(0))
            })
        })
        .collect()
}

/// What an extension's finding action (`{query}` in) answers for a
/// query, as `parse` reads it, synchronously for the engine.
///
/// A linked extension's action answers at once; a process's later, its
/// answer kept a moment ([`ASKED_AGAIN`]) and the palette told to look
/// again (`Commands::refresh`), the last answer shown meanwhile.
fn asked<T: Clone + 'static>(
    list: ActionList,
    commands: &architect_commands::Commands,
    action: String,
    parse: impl Fn(Option<Vec<u8>>) -> Vec<T> + 'static,
) -> impl Fn(&str) -> Vec<T> + 'static {
    type Kept<T> = Rc<std::cell::RefCell<BTreeMap<String, (std::time::Instant, Vec<T>)>>>;
    let answers: Kept<T> = Rc::default();
    let pending: Rc<std::cell::RefCell<std::collections::BTreeSet<String>>> = Rc::default();
    let shown_last: Rc<std::cell::RefCell<Vec<T>>> = Rc::default();
    let parse = Rc::new(parse);
    let commands = *commands;
    move |query: &str| -> Vec<T> {
        if let Some((at, items)) = answers.borrow().get(query)
            && at.elapsed() < ASKED_AGAIN
        {
            return items.clone();
        }
        if pending.borrow().contains(query) {
            return shown_last.borrow().clone();
        }
        let (list, action, args) = (
            list.clone(),
            action.clone(),
            json!({ "query": query }).to_string(),
        );
        let mut run = Box::pin(async move {
            let caller = architect::action::ActionCaller::app()
                .with_invocation(Invocation::new(InvocationSource::Palette));
            list.run_json_async_with(&caller, &action, args.as_bytes())
                .await
        });
        let mut cx = std::task::Context::from_waker(futures::task::noop_waker_ref());
        if let std::task::Poll::Ready(out) = run.as_mut().poll(&mut cx) {
            let items = parse(out.ok());
            shown_last.borrow_mut().clone_from(&items);
            return items;
        }
        pending.borrow_mut().insert(query.to_owned());
        let (kept, asked, shown, parse, query) = (
            Rc::clone(&answers),
            Rc::clone(&pending),
            Rc::clone(&shown_last),
            Rc::clone(&parse),
            query.to_owned(),
        );
        spawn(async move {
            let items = parse(run.await.ok());
            asked.borrow_mut().remove(&query);
            shown.borrow_mut().clone_from(&items);
            kept.borrow_mut()
                .insert(query, (std::time::Instant::now(), items));
            commands.refresh();
        });
        shown_last.borrow().clone()
    }
}

/// How long an answer an extension's opener gave later is shown before it
/// is asked again.
const ASKED_AGAIN: std::time::Duration = std::time::Duration::from_secs(1);

use architect_commands::{OpenIn, OpenItem, Opener};

/// ⌘O's opener for an extension's `decl`: a query runs its `action`
/// (`{query}` in, `{items}` out, as [`asked`] gets it); an item picked
/// runs its `open`.
fn quick_opener(
    list: ActionList,
    commands: &architect_commands::Commands,
    opens: mpsc::UnboundedSender<(ActionList, String, String)>,
    decl: &contrib::OpenerDecl,
) -> Opener {
    let kind = decl.kind.clone();
    let parse_kind = kind.clone();
    let items = asked(list.clone(), commands, decl.action.clone(), move |out| {
        open_items(&parse_kind, out)
    });
    let (open, by_id) = if decl.open.is_empty() {
        (decl.action.clone(), false)
    } else {
        (decl.open.clone(), true)
    };
    let opened = move |item: &OpenItem, open_in: OpenIn| {
        let place = match open_in {
            OpenIn::Here => "here",
            OpenIn::Tab => "tab",
            OpenIn::Split => "split",
        };
        let key = if by_id { "id" } else { "open" };
        let mut args = serde_json::Map::new();
        args.insert(key.to_owned(), json!(item.id));
        // Where it goes, to an action that takes it (`in` in its params).
        let takes_in = list
            .info(&open)
            .and_then(|i| i.params_schema)
            .is_some_and(|p| p.contains("\"in\""));
        if takes_in {
            args.insert("in".to_owned(), json!(place));
        }
        let args = Value::Object(args).to_string();
        let _ = opens.unbounded_send((list.clone(), open.clone(), args));
    };
    Opener::new(kind, decl.title.clone(), items, opened)
}

/// An opener action's answer (`{items: [{id, title, detail?, recency?}]}`)
/// as ⌘O's items: the palette sorts them by its match, then `recency`
/// (higher first), then title.
fn open_items(kind: &str, out: Option<Vec<u8>>) -> Vec<OpenItem> {
    let answer: Value = out
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(Value::Null);
    let list = answer
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    list.iter()
        .filter_map(|v| {
            let text = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default();
            let (id, title) = (text("id"), text("title"));
            (!id.is_empty()).then(|| {
                OpenItem::new(kind, id, if title.is_empty() { id } else { title })
                    .detail(text("detail"))
                    .recency(v.get("recency").and_then(Value::as_u64).unwrap_or(0))
            })
        })
        .collect()
}

/// The popups and overlays extensions opened, over the window: mount it
/// once, last, at the root.
#[component]
pub fn ExtensionOverlays(ui: ExtensionUi) -> Element {
    let layers = (ui.layers)();
    rsx! {
        crate::window_ui::ExtensionWindows { ui }
        crate::toolbar_editor::ToolbarEditorLayer { ui }
        crate::host_ui_view::HostUiLayer { host: ui.host() }
        for layer in layers {
            {
                let open = layer.open;
                let kind = open.kind.clone();
                let popup = open.placement == ViewPlacement::Popup;
                let (w, h) = (open.width.unwrap_or(480.0), open.height.unwrap_or(360.0));
                let close = kind.clone();
                rsx! {
                    div { key: "{kind}",
                        style: if popup {
                            "position: absolute; inset: 0; z-index: 80; display: flex; align-items: center; \
                             justify-content: center; background: rgba(0,0,0,0.45);"
                        } else {
                            "position: absolute; top: 72px; right: 24px; z-index: 80;"
                        },
                        onclick: move |_| if popup { ui.close_layer(&close) },
                        div {
                            style: "position: relative; width: {w}px; height: {h}px; display: flex; flex-direction: column; \
                                    border-radius: 12px; overflow: hidden; background: var(--card); \
                                    border: 1px solid var(--border); box-shadow: 0 16px 44px rgba(0,0,0,0.5);",
                            onclick: move |e| e.stop_propagation(),
                            div { style: "display: flex; align-items: center; height: 32px; padding: 0 10px; \
                                          font-size: 12px; color: var(--muted-foreground); border-bottom: 1px solid var(--border);",
                                span { style: "flex: 1;", "{open.title}" }
                                button {
                                    style: "border: none; background: transparent; color: var(--muted-foreground);",
                                    "aria-label": "Close",
                                    onclick: {
                                        let k = kind;
                                        move |_| ui.close_layer(&k)
                                    },
                                    "✕"
                                }
                            }
                            div { style: "flex: 1; min-height: 0; display: flex; flex-direction: column;",
                                ExtensionView { ui, extension: open.extension, view: open.view }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A view's page, as [`ViewPages`] is asked for it.
#[derive(Clone)]
pub struct PageAsk {
    pub extension: String,
    pub view: String,
    /// Whether it fills its pane (false: a slot, sized to its content).
    pub fill: bool,
    /// Its manifest's `page`, inside `dir`.
    pub page: Option<String>,
    /// The extension's folder, when it has one (a linked built-in has none).
    pub dir: Option<std::path::PathBuf>,
    /// What its runs may reach: the extension's capabilities.
    pub gate: architect_scripting::luau::Gate,
}

impl std::fmt::Debug for PageAsk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageAsk")
            .field("extension", &self.extension)
            .field("view", &self.view)
            .field("page", &self.page)
            .finish_non_exhaustive()
    }
}

/// Draws an extension's view from a page file when there is one for it.
///
/// `architect-pages` provides it; `None` leaves the view as it is.
/// Reading a signal inside it re-renders the view when that moves (an
/// override saved or deleted).
#[derive(Clone)]
pub struct ViewPages(pub Rc<dyn Fn(&PageAsk) -> Option<Element>>);

/// Extension `extension`'s view `view`: its linked-in component, its
/// process's remote view, or — when it is not loaded — a placeholder.
/// It fills its parent (a pane); `fill: false` sizes it to its content
/// (a slot of the shell's: a status bar, a ribbon).
#[component]
pub fn ExtensionView(
    ui: ExtensionUi,
    extension: String,
    view: String,
    #[props(default = true)] fill: bool,
    // A linked view's component as it is, in no box of its own (a
    // window's: the window is its box).
    #[props(default)] bare: bool,
) -> Element {
    let size = if fill {
        "flex: 1; min-height: 0;"
    } else {
        "flex-shrink: 0;"
    };
    let _ = (ui.revision)();
    let host = ui.host();
    let Some(loaded) = host.get(&extension) else {
        let name = extension;
        return rsx! {
            div { style: "{size} display: flex; align-items: center; justify-content: center; padding: 24px; \
                          font-size: 13px; color: var(--muted-foreground); text-align: center;",
                "{name} is not loaded: its view comes back when it does."
            }
        };
    };
    if let Some(pages) = try_consume_context::<ViewPages>() {
        let ask = PageAsk {
            page: loaded
                .manifest
                .views
                .iter()
                .find(|v| v.id == view)
                .and_then(|v| v.page.clone()),
            dir: loaded.dir.clone(),
            gate: crate::host::gate(&loaded.manifest),
            extension: extension.clone(),
            view: view.clone(),
            fill,
        };
        if let Some(page) = (pages.0)(&ask) {
            // A page is a sub-document, sized by its box: a slot's is a
            // bar's height, as a process view's.
            let boxed = if fill {
                "flex: 1; min-height: 0; display: flex; flex-direction: column;"
            } else {
                "flex-shrink: 0; height: 26px; display: flex; flex-direction: column;"
            };
            return rsx! {
                div { style: "{boxed}", {page} }
            };
        }
    }
    if let Some(root) = host.linked_view(&extension, &view) {
        // A list of one, so its key is diffed: a pane that goes from one
        // of its views to another (Projects' list to a project's page)
        // mounts the new one afresh, its hooks its own.
        let key = format!("{extension}/{view}");
        if bare {
            return rsx! {
                for key in std::iter::once(key) {
                    LinkedRoot { key: "{key}", root }
                }
            };
        }
        return rsx! {
            div { style: "{size} display: flex; flex-direction: column;",
                for key in std::iter::once(key) {
                    LinkedRoot { key: "{key}", root }
                }
            }
        };
    }
    #[cfg(all(feature = "remote", unix))]
    if let Some(processes) = try_use_context::<crate::process::Processes>() {
        // A remote document is sized by its box: a slot's is a bar's height.
        let remote = if fill {
            "flex: 1; min-height: 0; display: flex; flex-direction: column;"
        } else {
            "flex-shrink: 0; height: 26px; display: flex; flex-direction: column;"
        };
        return rsx! {
            div { style: "{remote}",
                crate::remote::RemoteView { processes, extension, view }
            }
        };
    }
    rsx! {
        div { style: "padding: 24px; font-size: 13px; color: var(--muted-foreground);",
            "{extension}'s view {view} is drawn by its process, which this app does not show."
        }
    }
}

/// A linked-in view's component, mounted.
#[component]
fn LinkedRoot(root: fn() -> Element) -> Element {
    root()
}
