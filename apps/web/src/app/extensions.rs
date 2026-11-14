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
        host.set_app_list((**app).clone());
        host.forward_actions_to(app);
        let mut extensions = Extensions::new(host, Installs::new(INSTALLS));
        let linked = architect_distribution::current()
            .map(architect_distribution::Distribution::extensions)
            .unwrap_or_default();
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
