//! The settings page, a built-in extension (#272): it owns the window ⌘,
//! opens (`APP_SETTINGS`, an app id the app allows it), a sheet over the
//! app, and the point `settings.sections`, whose items are the page's
//! sections: the app's own (Workspace, Appearance, …), and every
//! extension's (`[[sections]]`, sugar for an item of the point; the
//! extensions manager's among them).
//!
//! One [`SettingsSheet`] value runs it linked into an app
//! (`Extensions::with_linked`) or as its own process (`src/main.rs`, a
//! development copy made by `EXT_CLONE {"id": "settings"}`). Either way
//! its sections arrive through the host (`SETTINGS_SECTIONS`, the point's
//! delivery) and it draws them with the core's frame
//! (`architect_settings::SettingsSheet`): linked in over the app's values
//! and section pages; out of process over the values the host service
//! gives it (`APP_SETTINGS_VALUES`, `APP_SETTINGS_SET`), a section drawn
//! by another's view named, not drawn.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use architect_extensions::hostcall::HostRequest;
use architect_extensions::linked::{Builder, Extension, Handle};
use architect_extensions::manifest::{SectionDecl, section_of};
use architect_settings::{
    Registry, ScopedValues, SettingsChange, SettingsHandle, SettingsSheet as Sheet, SheetTheme,
    try_use_settings,
};
use dioxus::prelude::*;
use serde_json::{Value, json};

/// Its id.
pub const ID: &str = "settings";

/// Its manifest.
pub const MANIFEST: &str = include_str!("../extension.toml");

/// Its window's id.
pub const WINDOW: &str = "settings";

/// The settings page, as a `linked::Extension`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SettingsSheet;

/// The point's items as last delivered: its revision, and each item with
/// who gave it.
struct Delivered {
    revision: u64,
    items: Vec<(String, Value)>,
}

static DELIVERED: Mutex<Delivered> = Mutex::new(Delivered {
    revision: 0,
    items: Vec::new(),
});

/// Moves on every delivery kept: what its view looks at.
static CHANGES: AtomicU64 = AtomicU64::new(0);

static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

fn delivered() -> MutexGuard<'static, Delivered> {
    DELIVERED.lock().unwrap_or_else(PoisonError::into_inner)
}

fn handle() -> Option<Handle> {
    HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

/// Keeps a delivery (`{point, revision, items: [{from, value}]}`) unless
/// a newer one was kept: whether it was.
pub fn keep(delivery: &Value) -> bool {
    let revision = delivery
        .get("revision")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let items: Vec<(String, Value)> = delivery
        .get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|i| Some((i.get("from")?.as_str()?.to_owned(), i.get("value")?.clone())))
        .collect();
    let mut d = delivered();
    if revision < d.revision {
        return false;
    }
    d.revision = revision;
    d.items = items;
    drop(d);
    CHANGES.fetch_add(1, Ordering::Relaxed);
    true
}

/// The sections `items` (who gave each, and the item) make, in order.
///
/// An item that is not a section, or whose id another took first, is left
/// out. The app's own section drawn by a page of the app's (`view =
/// "page"`) is drawn by that page (linked in) or by its rows.
#[must_use]
pub fn registry(items: &[(String, Value)]) -> Registry {
    let mut r = Registry::new();
    for (from, value) in items {
        let Ok(mut decl) = serde_json::from_value::<SectionDecl>(value.clone()) else {
            continue;
        };
        let app_page = from == "app" && decl.view.as_deref() == Some("page");
        if app_page {
            decl.view = None;
        }
        let _ = r.register(section_of(from, &decl));
    }
    r
}

/// What the page lists: each section's id, title, group and who gave it.
#[must_use]
pub fn listed(items: &[(String, Value)]) -> Value {
    let r = registry(items);
    let from = |id: &str| {
        items
            .iter()
            .find(|(_, v)| v.get("id").and_then(Value::as_str) == Some(id))
            .map(|(f, _)| f.clone())
            .unwrap_or_default()
    };
    let sections: Vec<Value> = r
        .sections()
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "title": s.title,
                "group": s.group,
                "from": from(&s.id),
                "view": s.view,
                "settings": s.settings().count(),
            })
        })
        .collect();
    json!({ "sections": sections })
}

impl Extension for SettingsSheet {
    fn manifest(&self) -> &'static str {
        MANIFEST
    }

    fn build(&self, ext: &mut Builder) {
        *HANDLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(ext.handle());
        ext.view("sheet", SheetView)
            .action("SETTINGS_SECTIONS", |delivery| async move {
                Ok(json!({ "kept": keep(&delivery) }))
            })
            .action("SETTINGS_LIST", |_| async move {
                let items = delivered().items.clone();
                Ok(listed(&items))
            });
    }
}

/// Redraws the calling view whenever a delivery is kept.
fn use_changes() -> u64 {
    let mut seen = use_signal(|| CHANGES.load(Ordering::Relaxed));
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(Duration::from_millis(150)).await;
            let now = CHANGES.load(Ordering::Relaxed);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

/// Runs app action `action` through the host service.
async fn run(action: &str, args: Value) -> Result<Value, String> {
    let Some(host) = handle() else {
        return Err("not running".to_owned());
    };
    host.host(HostRequest::Run {
        action: action.to_owned(),
        args,
        ticket: None,
    })
    .await
    .map_err(|e| format!("{action}: {e}"))
}

/// A setting's value from JSON.
fn value_of(v: &Value) -> Option<architect_settings::Value> {
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

fn json_of(v: &architect_settings::Value) -> Value {
    match v {
        architect_settings::Value::Bool(b) => json!(b),
        architect_settings::Value::Int(i) => json!(i),
        architect_settings::Value::Float(f) => json!(f),
        architect_settings::Value::Text(t) => json!(t),
    }
}

/// The settings page in its window: the sections delivered, drawn with
/// the core's frame over the app's settings (linked in) or its own copy
/// of their values (out of process).
#[component]
fn SheetView() -> Element {
    let changes = use_changes();
    let mut sections = use_signal(|| registry(&delivered().items));
    use_effect(use_reactive!(|changes| {
        let _ = changes;
        sections.set(registry(&delivered().items));
    }));
    let app = try_use_settings();
    let own = app.is_none();
    let h = use_hook(move || {
        let base = app.unwrap_or_else(|| {
            SettingsHandle::new(
                Registry::new(),
                ScopedValues::default(),
                Some(Callback::new(|change: SettingsChange| {
                    for key in change.keys {
                        let value = change.values.set.get(&key).map(json_of);
                        spawn(async move {
                            let _ = run("APP_SETTINGS_SET", json!({ "key": key, "value": value }))
                                .await;
                        });
                    }
                })),
            )
        });
        base.with_registry(sections)
    });
    use_context_provider(move || h);
    // Out of process: the host's theme, and the values the app has now.
    let resolved = architect_extensions::guest::use_theme();
    let shell = use_signal(|| {
        <architect_shell::ShellTheme as architect_theme::FromResolved>::from_resolved(
            &resolved.clone().unwrap_or_default(),
        )
    });
    if own {
        use_context_provider(move || SheetTheme(shell));
    }
    {
        let mut shell = shell;
        use_effect(use_reactive!(|resolved| {
            if let Some(r) = resolved {
                shell.set(
                    <architect_shell::ShellTheme as architect_theme::FromResolved>::from_resolved(
                        &r,
                    ),
                );
            }
        }));
    }
    use_future(move || async move {
        if !own {
            return;
        }
        loop {
            if let Ok(answer) = run("APP_SETTINGS_VALUES", Value::Null).await {
                let mut scoped = ScopedValues::default();
                let registry = sections.peek().clone();
                if let Some(values) = answer.get("values").and_then(Value::as_object) {
                    for (key, v) in values {
                        if let Some(v) = value_of(v) {
                            let _ = scoped.set(&registry, key, v);
                        }
                    }
                }
                let mut values = h.values;
                if *values.peek() != scoped {
                    values.set(scoped);
                }
            }
            futures_timer::Delay::new(Duration::from_millis(500)).await;
        }
    });
    // Nothing open: the first section. Closed (its own close): the
    // window closes.
    let mut was_open = use_signal(|| false);
    use_effect(move || {
        let open = (h.open)().is_some();
        let has = !sections.read().sections().is_empty();
        if !open && !*was_open.peek() && has {
            h.show(None);
        }
        if !open && *was_open.peek() {
            if let Some(host) = handle() {
                spawn(async move {
                    let _ = host.window(WINDOW, false, Value::Null).await;
                });
            }
        }
        was_open.set(open);
    });
    if own {
        // Its own document: the frame's box, font and colours, which the
        // window's card gives a linked sheet.
        let t = shell();
        return rsx! {
            div {
                style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; overflow: hidden; \
                        background: {t.desk}; font-family: {t.font}; color: {t.ink};",
                Sheet {}
            }
        };
    }
    rsx! { Sheet {} }
}
