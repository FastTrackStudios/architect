//! The lab's themes: one `architect_theme::ThemeStore` for the whole app.
//! The shell, the architect-ui root and the paint panes all follow it; its
//! `THEME_*` actions are on the lab's Action List; the theme picker is
//! architect's built-in extension (`architect_theme_picker`, linked in
//! `extensions.rs`).
//!
//! The person's look is architect's Appearance section
//! (`architect_settings::use_appearance_section`), and the store's org,
//! person and device layers are kept as settings by architect
//! (`architect_settings::appearance::Persist::local`): this device's in
//! `<app dir>/appearance.json`, the person's in a synced settings
//! document (the lab has no account to sync it with, so the document is
//! a file, `<app dir>/person.loro`), the org's default from the made-up
//! orgs' settings ([`org_settings`]).
//!
//! In a window its themes folder is `<app dir>/themes`: the lab's shipped
//! themes are installed there (an edited one is kept), every file in it
//! is read over architect's built-in themes, and a saved change shows at
//! once. Headless (the guide's films and tests) it has the built-in
//! themes only, so a scenario never reads the person's own.

use std::rc::Rc;

use architect_settings::appearance::Persist;
use architect_theme::fs::{Shipped, ThemeDirs, ThemeWatch};
use architect_theme::{Scheme, ThemeHandle, ThemeStore};
use dioxus::prelude::*;

/// The lab's own themes, written into its themes folder.
const SHIPPED: &[Shipped] = &[
    Shipped {
        id: "lab-amber-dark",
        text: include_str!("../themes/lab-amber-dark.json"),
        previous: &[],
    },
    Shipped {
        id: "lab-paper-light",
        text: include_str!("../themes/lab-paper-light.json"),
        previous: &[],
    },
];

/// The lab's themes folder, in a window.
#[cfg(not(target_arch = "wasm32"))]
fn themes_dir() -> Option<std::path::PathBuf> {
    architect::settings::app_dir("architect-lab")
        .ok()
        .map(|dir| dir.join("themes"))
}

/// What keeps the lab's look, when it is kept (in a window): told when
/// the window's org changes (`Persist::set_org`).
pub type Kept = Rc<Option<Persist>>;

/// The store, given to everything below; with `files` (a window, or a
/// film's run on a folder of its own), read from the themes folder and
/// watched; in a window, its person and device layers kept
/// (`Persist::local`). Its actions go on `list`.
pub fn use_lab_theme(
    windowed: bool,
    files: bool,
    list: &architect::action::ActionList,
) -> (ThemeHandle, Kept) {
    let handle = architect_theme::use_theme_store(ThemeStore::default);
    let store = handle.store();
    // Kept for the app's life: dropping them stops the watcher and
    // writes what waits.
    let kept: (Rc<Option<ThemeWatch>>, Kept) = use_hook(|| {
        let _ = architect_theme::actions::register(list, &store);
        let _ = store.set(architect_theme::LayerKind::App, app_layer());
        let watch = files.then(|| watch_folder(&store)).flatten();
        let persist = windowed.then(|| keep(&store)).flatten();
        (Rc::new(watch), Rc::new(persist))
    });
    (handle, kept.1)
}

/// The lab's own tokens, as its `App` layer: the defaults of its views'
/// roles (the Keyflow editor's `editor.*`), expressions over the palette so
/// they follow the theme showing.
#[must_use]
pub fn app_layer() -> architect_theme::Layer {
    let layer = architect_theme::Layer::default();
    #[cfg(feature = "keyflow-lab")]
    let layer = crate::keyflow::EDITOR_TOKENS
        .iter()
        .fold(layer, |layer, (name, value)| layer.token(*name, *value));
    layer
}

#[cfg(not(target_arch = "wasm32"))]
fn watch_folder(store: &ThemeStore) -> Option<ThemeWatch> {
    let dir = themes_dir()?;
    let _ = architect_theme::fs::install(&dir, SHIPPED);
    let dirs = ThemeDirs::new().folder(&dir);
    dirs.install_into(store);
    architect_theme::fs::watch(store, &dirs).ok()
}

#[cfg(target_arch = "wasm32")]
fn watch_folder(_: &ThemeStore) -> Option<ThemeWatch> {
    None
}

/// Keeps the store's person and device layers, in a window; loads them
/// (and they show) first.
#[cfg(not(target_arch = "wasm32"))]
fn keep(store: &ThemeStore) -> Option<Persist> {
    Persist::local(
        store,
        "architect-lab",
        std::time::Duration::from_millis(250),
    )
}

#[cfg(target_arch = "wasm32")]
fn keep(_: &ThemeStore) -> Option<Persist> {
    None
}

/// What each of the lab's made-up orgs sets as its members' default
/// look: a real app has it from the org's settings on its server.
#[must_use]
pub fn org_settings(org: Option<&str>) -> architect_settings::OrgSettings {
    use architect_settings::appearance::{THEME, token_key, tokens};
    use architect_settings::{OrgSettings, Value};
    match org {
        Some("hp") => OrgSettings::new()
            .default_to(THEME, Value::Text("nord-light,nord-dark".into()))
            .default_to(token_key(tokens::RADIUS), Value::Text("0.375rem".into())),
        Some("fts") => OrgSettings::new().default_to(
            THEME,
            Value::Text("fasttrackstudio-light,fasttrackstudio-dark".into()),
        ),
        _ => OrgSettings::new(),
    }
}

/// The OS's light or dark, told to the store: at start, and whenever the
/// window hears it change. Mount it only in a window.
#[component]
pub fn SystemScheme() -> Element {
    use dioxus_native::winit::event::WindowEvent;
    use dioxus_native::winit::window::Theme;

    let handle = architect_theme::use_theme();
    let scheme = |theme: Theme| match theme {
        Theme::Light => Scheme::Light,
        Theme::Dark => Scheme::Dark,
    };
    use_hook(move || {
        let window =
            try_consume_context::<std::sync::Arc<dyn dioxus_native::winit::window::Window>>();
        if let Some(theme) = window.and_then(|w| w.theme()) {
            let _ = handle.set_system(scheme(theme));
        }
    });
    dioxus_native::use_window_event(move |event, _| {
        if let WindowEvent::ThemeChanged(theme) = event {
            let _ = handle.set_system(scheme(*theme));
        }
    });
    rsx! {}
}

/// The page's ground and the rows' feedback, in the theme's colours.
#[must_use]
pub fn page_css(resolved: &architect_theme::Resolved) -> String {
    format!(
        "html, body {{ margin: 0; padding: 0; background: {desk}; }}\
         button {{ cursor: pointer; }}\
         .lp:hover {{ box-shadow: inset 0 0 0 999px {hover}; }}\
         .lp:active {{ background-color: {active}; }}",
        desk = resolved.value("desk"),
        hover = resolved
            .color("ink")
            .alpha(if resolved.scheme().is_dark() {
                0.035
            } else {
                0.05
            })
            .to_css(),
        active = resolved.value("active"),
    )
}
