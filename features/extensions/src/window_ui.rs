//! Extensions' windows in the app's window (feature `ui`): the window
//! machinery the core keeps (`docs/content/architecture/extensions.md`,
//! "Windows, points and services").
//!
//! [`ExtensionWindows`] draws each open `[[windows]]` (`crate::windows`):
//! a `sheet` as a scrim and a card over the main window (on a phone, the
//! whole screen) holding the extension's view; a `window` as an OS window
//! of its own, through the workspace's pop-out machinery
//! (`architect_workspace::Popout::page`) where the platform gives one
//! (`architect_workspace::Windows` in context), else as a sheet. Either
//! way the view is drawn as any of its views is ([`ExtensionView`]:
//! linked in, a process's or a WASI component's remote document, a page),
//! and follows the theme live: a sheet is in the app's document; an OS
//! window carries the shell and the theme's `:root` sheet into its own.
//!
//! [`use_settings_window`] joins the app's settings (`architect_settings`'s
//! [`SettingsHandle`]) to the settings extension's window: the app's own
//! sections as its items of `settings.sections`, `SettingsHandle::show`
//! opening the window that `APP_SETTINGS` opens, its closing closing the
//! page, and the values for a sheet drawn out of process
//! (`APP_SETTINGS_VALUES`, `APP_SETTINGS_SET`).

use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use architect::action::{ActionCaller, ActionError, ActionFuture, ActionInfo, Effect};
use architect_settings::{Owner, try_use_settings};
use architect_shell::{ShellTheme, try_use_shell};
use dioxus::prelude::*;
use futures::StreamExt as _;
use futures::channel::mpsc;
use serde_json::{Value, json};

use crate::host::Host;
use crate::interface::{ExtensionUi, ExtensionView};
use crate::interop::{SETTINGS_SECTIONS, WindowKind};
use crate::windows::OpenWindow;

/// The action that opens the settings page.
pub const APP_SETTINGS: &str = "APP_SETTINGS";

/// The open windows, as a signal that moves when one opens or closes.
pub(crate) fn use_open_windows(host: &Host) -> Signal<Vec<OpenWindow>> {
    let mut open = use_signal(|| host.open_windows());
    let h = host.clone();
    use_future(move || {
        let h = h.clone();
        async move {
            let (tx, mut rx) = mpsc::unbounded::<()>();
            h.on_windows(move || {
                let _ = tx.unbounded_send(());
            });
            // What opened before it listened.
            let now = h.open_windows();
            if *open.peek() != now {
                open.set(now);
            }
            while rx.next().await.is_some() {
                while rx.try_recv().is_ok() {}
                open.set(h.open_windows());
            }
        }
    });
    open
}

fn shell_theme() -> ShellTheme {
    try_use_shell().map_or_else(
        || ShellTheme::from_resolved(&architect_theme::use_resolved()),
        |s| s.theme,
    )
}

fn phone() -> bool {
    try_use_shell().is_some_and(|s| s.form() == architect_shell::FormFactor::Phone)
}

/// Every open extension window: mount it once at the root, over the
/// window (`ExtensionOverlays` does).
#[component]
pub fn ExtensionWindows(ui: ExtensionUi) -> Element {
    let open = ui.windows();
    let os = try_consume_context::<architect_workspace::Windows>();
    rsx! {
        for w in open {
            if w.window.kind == WindowKind::Window && os.is_some() {
                OsWindow { key: "{w.key()}", ui, open: w }
            } else {
                Sheet { key: "{w.key()}", ui, open: w }
            }
        }
    }
}

/// A sheet over the main window: a scrim (a click on it closes), a card
/// of the window's size holding its view.
#[component]
fn Sheet(ui: ExtensionUi, open: OpenWindow) -> Element {
    let t = shell_theme();
    let (ext, id) = (open.extension.clone(), open.window.id.clone());
    let close = move |_| {
        ui.host().close_window(&ext, &id);
    };
    let view = rsx! {
        ExtensionView { ui, extension: open.extension.clone(), view: open.window.view.clone(), bare: true }
    };
    if phone() {
        return rsx! {
            div {
                style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {architect_shell::layer::POPOVER}; \
                        display: flex; flex-direction: column; background: {t.desk}; font-family: {t.font}; color: {t.ink};",
                {view}
            }
        };
    }
    let size = match (open.window.width, open.window.height) {
        (None, None) => "width: 92%; max-width: 1100px; height: 86%; max-height: 760px;".to_owned(),
        (w, h) => format!(
            "width: {}px; max-width: 92%; height: {}px; max-height: 86%;",
            w.unwrap_or(720.0),
            h.unwrap_or(560.0)
        ),
    };
    rsx! {
        div {
            style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {architect_shell::layer::POPOVER}; \
                    background: {t.scrim}; display: flex; align-items: center; justify-content: center;",
            onclick: close,
            div {
                style: "{size} display: flex; overflow: hidden; \
                        border-radius: 14px; background: {t.desk}; border: 1px solid {t.rule}; box-shadow: {t.shadow_float}; \
                        font-family: {t.font}; color: {t.ink};",
                onclick: move |e| e.stop_propagation(),
                {view}
            }
        }
    }
}

/// An OS window of its own: opened through the platform's windows when it
/// mounts, closed when it unmounts (the window closed in the host); the
/// person closing it closes it in the host.
#[component]
fn OsWindow(ui: ExtensionUi, open: OpenWindow) -> Element {
    let os = try_consume_context::<architect_workspace::Windows>();
    let carried = use_hook(|| {
        architect_workspace::Bridge::new()
            .context::<Signal<architect_shell::Shell>>()
            .context::<architect_theme::RootCss>()
            .context::<ExtensionUi>()
    });
    let id = architect_workspace::WindowId(format!("ext-{}", open.key().replace('/', "-")));
    {
        let (id, os) = (id.clone(), os.clone());
        use_hook(move || {
            let Some(os) = os else { return };
            let (ext, view) = (open.extension.clone(), open.window.view.clone());
            let draw: Rc<dyn Fn() -> Element> = Rc::new(move || {
                rsx! { WindowPage { ui, extension: ext.clone(), view: view.clone() } }
            });
            let (ext, wid) = (open.extension.clone(), open.window.id.clone());
            // Placed by the platform, at its size (720 × 560 unless it says).
            let bounds = None;
            let title = if open.window.title.is_empty() {
                open.window.id.clone()
            } else {
                open.window.title.clone()
            };
            os.0.open(architect_workspace::Popout::page(
                id,
                title,
                bounds,
                carried,
                draw,
                Rc::new(move || {
                    ui.host().close_window(&ext, &wid);
                }),
            ));
        });
    }
    use_drop(move || {
        if let Some(os) = os {
            os.0.close(&id);
        }
    });
    rsx! {}
}

/// An OS window's document: the ground in the theme's colours, the
/// theme's `:root` sheet for `var(--…)`, and the view filling it.
#[component]
fn WindowPage(ui: ExtensionUi, extension: String, view: String) -> Element {
    let t = shell_theme();
    let root = architect_theme::try_use_root_css().map(|r| r());
    let ground = format!(
        "html, body {{ margin: 0; padding: 0; height: 100%; background: {}; }}",
        t.desk
    );
    rsx! {
        document::Style { {ground} }
        if let Some(css) = root {
            document::Style { {css} }
        }
        div {
            style: "position: relative; width: 100vw; height: 100vh; display: flex; overflow: hidden; \
                    background: {t.desk}; color: {t.ink}; font-family: {t.font}; font-size: 13px;",
            ExtensionView { ui, extension, view, bare: true }
        }
    }
}

// ---- the settings page's window ---------------------------------------

/// A JSON value as a setting's.
fn setting_value(v: &Value) -> Option<architect_settings::Value> {
    match v {
        Value::Bool(b) => Some(architect_settings::Value::Bool(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(architect_settings::Value::Int)
            .or_else(|| n.as_f64().map(architect_settings::Value::Float)),
        Value::String(t) => Some(architect_settings::Value::Text(t.clone())),
        _ => None,
    }
}

/// A setting's value as JSON.
fn setting_json(v: &architect_settings::Value) -> Value {
    match v {
        architect_settings::Value::Bool(b) => json!(b),
        architect_settings::Value::Int(i) => json!(i),
        architect_settings::Value::Float(f) => json!(f),
        architect_settings::Value::Text(t) => json!(t),
    }
}

/// What a settings sheet out of process reads and writes: every setting's
/// value now, and the writes it asked for.
#[derive(Default)]
struct Shared {
    now: Mutex<BTreeMap<String, Value>>,
    writes: Mutex<Vec<(String, Option<Value>)>>,
}

fn info(id: &str, name: &str, effect: Effect, description: &str, params: &str) -> ActionInfo {
    ActionInfo {
        id: id.to_owned(),
        display_name: name.to_owned(),
        description: description.to_owned(),
        category: "App".to_owned(),
        group: "Settings".to_owned(),
        shortcut: String::new(),
        effect,
        params_schema: (!params.is_empty()).then(|| params.to_owned()),
        result_schema: None,
        enabled: true,
        disabled_reason: None,
        checked: None,
    }
}

/// Joins the app's settings to the settings extension's window: call it
/// where the app's [`SettingsHandle`] and `host` are (`use_extension_ui`
/// does). Without a settings handle in context it does nothing.
pub(crate) fn use_settings_window(host: &Host, open: Signal<Vec<OpenWindow>>) {
    let handle = try_use_settings();
    let shared = use_hook(|| Arc::new(Shared::default()));
    // The app's own sections, as its items of `settings.sections` (an
    // extension's come from its manifest); a page of the app's own
    // (Appearance) is named so the sheet draws it with the app's page.
    {
        let host = host.clone();
        use_effect(move || {
            let Some(h) = handle else { return };
            let registry = h.registry.read();
            let pages = h.pages.read();
            let items: Vec<Value> = registry
                .sections()
                .iter()
                .filter(|s| s.owner == Owner::App)
                .filter_map(|s| {
                    serde_json::to_value(crate::manifest::decl_of(s, pages.contains_key(&s.id)))
                        .ok()
                })
                .collect();
            drop((registry, pages));
            host.contribute_app(SETTINGS_SECTIONS, items);
        });
    }
    // The values, for a sheet out of process.
    {
        let shared = Arc::clone(&shared);
        use_effect(move || {
            let Some(h) = handle else { return };
            let registry = h.registry.read();
            let now = h.values.read();
            let mut out = BTreeMap::new();
            for section in registry.sections() {
                for setting in section.settings() {
                    if let Some(v) = now.get(&registry, &setting.key) {
                        out.insert(setting.key.clone(), setting_json(&v));
                    }
                }
            }
            *shared.now.lock().unwrap_or_else(PoisonError::into_inner) = out;
        });
    }
    {
        let shared = Arc::clone(&shared);
        use_future(move || {
            let shared = Arc::clone(&shared);
            async move {
                let Some(h) = handle else { return };
                loop {
                    futures_timer::Delay::new(std::time::Duration::from_millis(100)).await;
                    let queued: Vec<_> = std::mem::take(
                        &mut *shared.writes.lock().unwrap_or_else(PoisonError::into_inner),
                    );
                    for (key, value) in queued {
                        match value.as_ref().and_then(setting_value) {
                            Some(v) => h.set(&key, v),
                            None => h.reset(&key),
                        }
                    }
                }
            }
        });
    }
    {
        let (list, shared) = (host.list().clone(), Arc::clone(&shared));
        use_hook(move || {
            if handle.is_none() {
                return;
            }
            let now = Arc::clone(&shared);
            let _ = list.register_forwarded_with(
                &info(
                    "APP_SETTINGS_VALUES",
                    "Settings: every value",
                    Effect::ReadOnly,
                    "Every setting's value now, by key (its default when not set): what a settings \
                     sheet drawn out of process shows.",
                    "",
                ),
                Arc::new(move |_: &ActionCaller, _: &[u8]| {
                    let values = now
                        .now
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .clone();
                    let out: ActionFuture = Box::pin(async move {
                        Ok(json!({ "values": values }).to_string().into_bytes())
                    });
                    Ok(out)
                }),
            );
            let writes = Arc::clone(&shared);
            let _ = list.register_forwarded_with(
                &info(
                    "APP_SETTINGS_SET",
                    "Settings: set a value",
                    Effect::Mutating,
                    "Sets setting `key` to `value` in its scope's store, as the settings page does \
                     (a value it does not take, or one the org locked, is let go); no `value` puts \
                     its default back.",
                    r#"{"type":"object","properties":{"key":{"type":"string"},"value":{}},"required":["key"]}"#,
                ),
                Arc::new(move |_: &ActionCaller, args: &[u8]| {
                    let a: Value = serde_json::from_slice(args).map_err(|e| ActionError::BadArgs {
                        id: "APP_SETTINGS_SET".to_owned(),
                        message: e.to_string(),
                        expected: None,
                    })?;
                    let key = a.get("key").and_then(Value::as_str).unwrap_or_default().to_owned();
                    let value = a.get("value").cloned().filter(|v| !v.is_null());
                    writes
                        .writes
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .push((key, value));
                    let out: ActionFuture = Box::pin(async move { Ok(b"null".to_vec()) });
                    Ok(out)
                }),
            );
        });
    }
    // `SettingsHandle::show` (a search hit, the manager's MANAGER_SHOW)
    // opens the window `APP_SETTINGS` opens; closing it closes the page.
    {
        let host = host.clone();
        let mut wanted = use_signal(|| false);
        use_effect(move || {
            let Some(h) = handle else { return };
            let want = (h.open)().is_some();
            let window =
                settings_decl(&host).filter(|_| settings_window(&host, &open.read()).is_some());
            let before = *wanted.peek();
            match (want, window.is_some(), before) {
                // Shown while closed: open the window.
                (true, false, false) => {
                    if let Some((ext, id)) = settings_decl(&host) {
                        let _ = host.open_window(&ext, &id, Value::Null);
                    }
                }
                // The window closed (the person, its extension): the page
                // closes.
                (true, false, true) => {
                    h.close();
                    wanted.set(false);
                    return;
                }
                // The page closed (its own close): the window closes.
                (false, true, true) => {
                    if let Some((ext, id)) = window {
                        host.close_window(&ext, &id);
                    }
                }
                _ => {}
            }
            wanted.set(want);
        });
    }
    // The window opened (⌘,, the palette, an agent): the page shows the
    // section its state names, or stays where it was, or the first.
    {
        let host = host.clone();
        let mut last = use_signal(|| 0_u64);
        use_effect(move || {
            let Some(h) = handle else { return };
            let Some(w) = settings_window(&host, &open.read()) else {
                return;
            };
            if *last.peek() == w.opened {
                return;
            }
            last.set(w.opened);
            let section = w
                .state
                .get("section")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let setting = w
                .state
                .get("setting")
                .and_then(Value::as_str)
                .map(str::to_owned);
            if let Some(key) = setting {
                h.show_setting(&key);
            } else if section.is_some() || h.open.peek().is_none() {
                h.show(section.as_deref());
            }
        });
    }
}

/// The window `APP_SETTINGS` opens: its extension and id.
fn settings_decl(host: &Host) -> Option<(String, String)> {
    host.window_decls()
        .into_iter()
        .find(|(_, w)| w.opens == APP_SETTINGS)
        .map(|(ext, w)| (ext, w.id))
}

/// That window, when it is open.
fn settings_window(host: &Host, open: &[OpenWindow]) -> Option<OpenWindow> {
    let (ext, id) = settings_decl(host)?;
    open.iter()
        .find(|w| w.extension == ext && w.window.id == id)
        .cloned()
}
