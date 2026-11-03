//! The lab's themes: one `architect_theme::ThemeStore` for the whole app.
//! The shell, the architect-ui root and the paint panes all follow it; its
//! `THEME_*` actions are on the lab's Action List; ⌘K has a theme picker.
//!
//! In a window its themes folder is `<app dir>/themes`: the lab's shipped
//! themes are installed there (an edited one is kept), every file in it
//! is read over architect's built-in themes, and a saved change shows at
//! once. Headless (the guide's films and tests) it has the built-in
//! themes only, so a scenario never reads the person's own.

use std::rc::Rc;

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

/// The store, given to everything below; in a window, read from the
/// themes folder and watched. Its actions go on `list`.
pub fn use_lab_theme(windowed: bool, list: &architect::action::ActionList) -> ThemeHandle {
    let handle = architect_theme::use_theme_store(ThemeStore::default);
    let store = handle.store();
    // Kept for the app's life: dropping it stops the watcher.
    let _watch: Rc<Option<ThemeWatch>> = use_hook(|| {
        let _ = architect_theme::actions::register(list, &store);
        Rc::new(windowed.then(|| watch_folder(&store)).flatten())
    });
    handle
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

/// ⌘K's theme picker: "Choose a theme…" opens the palette on the "Theme"
/// group, one command per theme (the ones known when the lab started;
/// `THEME_LIST` / `THEME_SET` reach every one), and "Light or dark".
pub fn use_theme_commands(handle: ThemeHandle, commands: &architect_commands::Commands) {
    let commands = commands.clone();
    let showing = handle.resolved().read().id().to_owned();
    let themes = use_hook(|| handle.store().themes());
    architect_commands::use_command(
        architect_commands::Command::new("LAB_THEME_CHOOSE", "Choose a theme…").group("View"),
        move || commands.show_group("Theme", "Choose a theme"),
    );
    architect_commands::use_command(
        architect_commands::Command::new("LAB_THEME_SCHEME", "Light or dark").group("View"),
        move || {
            let store = handle.store();
            let next = match store.current().scheme() {
                Scheme::Dark => architect_theme::Appearance::Light,
                Scheme::Light => architect_theme::Appearance::Dark,
            };
            let _ = store.update(architect_theme::LayerKind::Pick, |l| {
                l.appearance = Some(next);
            });
        },
    );
    for info in themes {
        let id = info.id.clone();
        architect_commands::use_command(
            architect_commands::Command::new(
                format!("LAB_THEME_{}", info.id.to_uppercase().replace('-', "_")),
                info.label.clone(),
            )
            .group("Theme")
            .checked(showing == info.id),
            move || pick(&handle.store(), &id),
        );
    }
}

/// Picks `id` so it shows now: its half of the pair, and the appearance
/// its scheme when the other one is showing.
fn pick(store: &ThemeStore, id: &str) {
    let _ = store.pick(id);
    let scheme = store.registry().scheme_of(id);
    if let Some(scheme) = scheme.filter(|s| *s != store.current().scheme()) {
        let _ = store.update(architect_theme::LayerKind::Pick, |l| {
            l.appearance = Some(match scheme {
                Scheme::Light => architect_theme::Appearance::Light,
                Scheme::Dark => architect_theme::Appearance::Dark,
            });
        });
    }
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
