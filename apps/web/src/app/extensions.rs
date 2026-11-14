//! Linked extensions in a browser (#495, W1): the distribution's
//! compiled-in extensions run here as on a desk and a phone.
//!
//! The same host the desktop app runs (`architect_extensions::host::Host`,
//! built for the browser without Luau, which is C++) loads each extension
//! the distribution links in ([`architect_distribution::current`]), and
//! the core every build has that a browser can run: the Command Palette
//! and the theme picker.
//! Their actions go onto the app's Action List (⌘K, and what a frame's or
//! a page's host call runs), their settings sections onto the settings
//! page, their views into the workspace's kinds (`ExtensionUi::views`),
//! drawn as Dioxus components in the app's own document.
//!
//! What a browser does not do here: nothing is installed from a folder
//! (there is none), no thread runs the host's clock or hears its events
//! (hooks fire from the app's events only where something raises them),
//! and an action's work runs on the page's one thread
//! (`spawn_local`).

use std::sync::Arc;

use architect::action::ActionList;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::Host;
use architect_extensions::install::Installs;
use architect_extensions::interface::{ExtensionUi, use_extension_ui};
use architect_extensions::linked::Extension;
use architect_extensions::ui::use_extension_sections;
use dioxus::prelude::*;

/// Where the host keeps installs: a name only (a browser has no folder;
/// reading it finds nothing).
const INSTALLS: &str = "/architect/extensions";

/// The extensions every browser build links beside the distribution's:
/// the core a browser runs, the Command Palette and the theme picker
/// (Navigator and the settings page are the web app's own; the playhead
/// is drawn by the app's Now Playing bar).
fn core() -> Vec<&'static dyn Extension> {
    vec![
        // ⌘K and ⌘O, drawn in the shell's palette slot (#381), as on a
        // desk.
        &architect_command_palette::CommandPalette,
        &architect_theme_picker::ThemePicker,
    ]
}

/// Starts the host over the app's list `app`, with the distribution's
/// linked extensions and the core loaded, and puts what they add into
/// the window. Call it once in the app's root, under the theme and the
/// settings providers.
pub fn use_web_extensions(app: &Arc<ActionList>) -> ExtensionUi {
    let sections = use_extension_sections();
    // The extensions' own list, in ⌘K (as on a desk:
    // `apps/ui/src/extensions.rs`), each action forwarded onto the app's
    // too, where a frame's, a page's and a test's calls find it.
    let list = use_hook(|| Arc::new(ActionList::new()));
    architect_commands::use_action_list(Arc::clone(&list));
    let themes = architect_theme::try_use_theme().map(|t| t.store());
    // Extensions' settings, as the settings page has them (kept in this
    // browser by the page).
    let values = architect_extensions::ui::use_page_setting_values();
    let web = try_use_context::<super::vault::WebVault>();
    use_hook(|| {
        APP_RUNTIME.with(|r| *r.borrow_mut() = Some(dioxus::core::Runtime::current()));
    });
    let extensions = use_hook(|| {
        let mut host = Host::new((*list).clone(), sections.clone())
            // Work no one awaits (a hook's) on the page's one thread, as a
            // task of the app's runtime: what it runs may read and write
            // the app's signals.
            .with_spawn(spawn_in_app);
        if let Some(store) = &themes {
            host = host.with_themes(store.clone());
        }
        // Consent in this tab, none given: an installed extension reaches
        // nothing until a person grants it (built-ins are not narrowed).
        host = host.with_grants(architect_extensions::consent::Grants::in_memory());
        host = host.with_setting_values(values.clone());
        // Their data: kept in this browser per org; the demo keeps
        // nothing past the tab, as it says.
        if let Some(org) = web.as_ref().and_then(|w| w.org.as_ref()) {
            host = host.with_data_store(LocalData { scope: org.slug() });
        } else if probe_asked() {
            // The demo's probe keeps it too, under a scope of its own, so
            // a browser test sees the same store survive a reload.
            host = host.with_data_store(LocalData {
                scope: "demo-probe".to_owned(),
            });
        }
        host.set_app_list((**app).clone());
        host.forward_actions_to(app);
        let mut extensions = Extensions::new(host, Installs::new(INSTALLS));
        let mut linked = architect_distribution::current()
            .map(architect_distribution::Distribution::extensions)
            .unwrap_or_default();
        // The demo's test probe, asked for (`?demo&probe`).
        if web.as_ref().is_some_and(|w| w.org.is_none()) && probe_asked() {
            linked.push(&Probe);
        }
        for extension in linked.into_iter().chain(core()) {
            match extensions.clone().with_linked(extension) {
                Ok(with) => extensions = with,
                Err(error) => report(&format!("a compiled-in extension did not load: {error}")),
            }
        }
        let _ = extensions.install_actions();
        let _ = extensions.install_contribution_actions();
        for event in extensions.sync() {
            if let architect_extensions::host::Event::Failed { id, error } = event {
                report(&format!("{id} did not load: {error}"));
            }
        }
        mark_loaded(&extensions.host().ids());
        extensions
    });
    // Scheduled hooks (`every`, `at`) and idle background extensions'
    // stops: what `Host::run_clock`'s thread does on a desk, on the page's
    // timer here.
    {
        let host = extensions.host().clone();
        use_future(move || {
            let host = host.clone();
            async move {
                loop {
                    futures_timer::Delay::new(std::time::Duration::from_secs(1)).await;
                    let _ = host.tick();
                }
            }
        });
    }
    // The web vault as their documents, and the person's own edits as
    // `doc.*` on the app's list, where the host hears them (#495 H1, H2).
    use_hook(|| {
        let Some(web) = web.clone() else { return };
        let host = extensions.host().clone();
        let docs = match &web.org {
            None => super::documents::WebDocuments::demo(web.source.clone()),
            Some(org) => {
                let (tx, mut rx) =
                    futures_channel::mpsc::unbounded::<super::documents::NodeWrite>();
                let org = org.clone();
                // In order, one at a time, from the page's thread.
                wasm_bindgen_futures::spawn_local(async move {
                    use futures_util::StreamExt as _;
                    while let Some(w) = rx.next().await {
                        if let Err(e) = org.write(&w.path, w.text).await {
                            org.report(format!("{} was not saved on the node: {e}", w.path));
                        }
                    }
                });
                super::documents::WebDocuments::org(web.source.clone(), tx)
            }
        };
        let events = docs.raise_on(app);
        host.set_documents(std::sync::Arc::new(docs));
        // Kept for the app's life.
        std::mem::forget(events);
    });
    // The app's events (the person's document edits among them) to the
    // host's hooks: what `Host::watch`'s thread does on a desk, on the
    // page's timer here.
    {
        let host = extensions.host().clone();
        let app = Arc::clone(app);
        use_future(move || {
            let (host, app) = (host.clone(), Arc::clone(&app));
            async move {
                let mut since = app.latest();
                loop {
                    futures_timer::Delay::new(std::time::Duration::from_millis(100)).await;
                    let (events, next) = app.events_since(since, 64, std::time::Duration::ZERO);
                    since = next;
                    for e in events {
                        if let architect::action::EventKind::Changed { topic, detail } = e.kind {
                            let _ = host.emit_changed(&topic, &detail);
                        }
                    }
                }
            }
        });
    }
    // The demo's probe, for the browser tests (`?demo&probe` only).
    use_hook(|| {
        if web.as_ref().is_some_and(|w| w.org.is_none()) && probe_asked() {
            // On the host's own list, where `EXT_PAGE_CALL` is.
            install_probe(Arc::clone(&list));
        }
    });
    // Each linked extension's root context, for its views below.
    use_hook(|| {
        for (_, built) in extensions.host().linked() {
            built.provide_context();
        }
    });
    use_extension_ui(extensions.host().clone(), Arc::clone(app))
}

/// Why an extension did not load, on the browser's console (the web app
/// has no log of its own yet).
fn report(why: &str) {
    web_sys::console::warn_1(&format!("extensions: {why}").into());
}

/// The extensions running, on the page's root as `data-extensions`
/// (space-separated ids): what a test and a person's devtools read.
fn mark_loaded(ids: &[String]) {
    if let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    {
        let _ = root.set_attribute("data-extensions", &ids.join(" "));
    }
}

thread_local! {
    /// The app's Dioxus runtime, for work the host spawns: a browser has
    /// one thread, so the host's `Send` spawner finds it here.
    static APP_RUNTIME: std::cell::RefCell<Option<std::rc::Rc<dioxus::core::Runtime>>> =
        const { std::cell::RefCell::new(None) };
}

/// Runs `work` as a task at the app's root (`spawn_forever`), inside its
/// runtime; before the app runs, on the page's thread alone.
fn spawn_in_app(work: architect::action::ActionFuture) {
    let runtime = APP_RUNTIME.with(|r| r.borrow().clone());
    match runtime {
        Some(rt) => rt.in_scope(ScopeId::ROOT, || {
            dioxus::core::spawn_forever(async move {
                let _ = work.await;
            });
        }),
        None => wasm_bindgen_futures::spawn_local(async move {
            let _ = work.await;
        }),
    }
}

/// Extensions' data in this browser's `localStorage`, one entry each
/// (`architect.extension-data.<scope>.<id>`), `scope` the org's slug: what
/// `Host::with_data_root` is on a desk (#495 H4). A private window or
/// cleared site data starts empty.
struct LocalData {
    scope: String,
}

impl LocalData {
    fn key(&self, id: &str) -> String {
        format!("architect.extension-data.{}.{id}", self.scope)
    }

    fn storage() -> Option<web_sys::Storage> {
        web_sys::window()?.local_storage().ok().flatten()
    }
}

impl architect_extensions::hostapi::DataStore for LocalData {
    fn load(&self, id: &str) -> Option<std::collections::BTreeMap<String, serde_json::Value>> {
        let text = Self::storage()?.get_item(&self.key(id)).ok().flatten()?;
        serde_json::from_str(&text).ok()
    }

    fn keep(
        &self,
        id: &str,
        data: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        let store = Self::storage().ok_or("this browser keeps no storage for the app")?;
        let text = serde_json::to_string(data).map_err(|e| e.to_string())?;
        store
            .set_item(&self.key(id), &text)
            .map_err(|_| "this browser's storage refused it (full, or private)".to_owned())
    }
}

/// Whether the page asked for the demo's probe (`?demo&probe`).
fn probe_asked() -> bool {
    web_sys::window()
        .and_then(|w| w.location().search().ok())
        .is_some_and(|q| q.trim_start_matches('?').split('&').any(|p| p == "probe"))
}

/// `window.architectProbe(id, request)`: one host-service call
/// (`EXT_PAGE_CALL`, a `HostRequest` as JSON) as extension `id`, checked
/// against its capabilities as any extension's is, its answer as JSON.
/// What the browser tests drive the document API and extension data with
/// (`apps/web/test/app-documents.test.mjs`). Only in the demo, which holds
/// no credentials and keeps nothing past the tab, and only when the page
/// was opened with `probe`; the signed-in app and a share link never have
/// it.
fn install_probe(app: Arc<ActionList>) {
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::prelude::Closure;
    let Some(window) = web_sys::window() else {
        return;
    };
    let call = Closure::<dyn Fn(String, String) -> js_sys::Promise>::new(
        move |id: String, request: String| {
            let app = Arc::clone(&app);
            wasm_bindgen_futures::future_to_promise(async move {
                let who = architect::action::Principal::Extension {
                    id,
                    on_behalf_of: None,
                };
                app.run_json_async_as(
                    &who,
                    architect_extensions::host::PAGE_CALL,
                    request.as_bytes(),
                )
                .await
                .map(|out| String::from_utf8_lossy(&out).into_owned().into())
                .map_err(|e| e.to_string().into())
            })
        },
    );
    let _ = js_sys::Reflect::set(
        &window,
        &"architectProbe".into(),
        call.as_ref().unchecked_ref(),
    );
    call.forget();
    // `window.architectProbeHeard()`: the document events the probe heard,
    // as JSON, in order.
    let heard = Closure::<dyn Fn() -> String>::new(|| {
        HEARD.with(|h| serde_json::Value::Array(h.borrow().clone()).to_string())
    });
    let _ = js_sys::Reflect::set(
        &window,
        &"architectProbeHeard".into(),
        heard.as_ref().unchecked_ref(),
    );
    heard.forget();
}

thread_local! {
    /// The document events the probe heard (its `WEB_PROBE_HEARD` hooks).
    static HEARD: std::cell::RefCell<Vec<serde_json::Value>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// The demo's probe extension (`probe.toml`): it may read and write the
/// documents, and records each document event it hears.
struct Probe;

impl Extension for Probe {
    fn manifest(&self) -> &'static str {
        include_str!("probe.toml")
    }

    fn build(&self, ext: &mut architect_extensions::linked::Builder) {
        // A hook's action is told the event in where it was invoked
        // from (`{source: "hook", event, …}`).
        ext.action_in("WEB_PROBE_HEARD", |args, cx| async move {
            HEARD.with(|h| {
                h.borrow_mut()
                    .push(serde_json::json!({ "args": args, "from": cx }));
            });
            Ok(serde_json::Value::Null)
        });
    }
}
