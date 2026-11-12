//! The theme in a browser: one store, its `THEME_*` actions, the app's
//! own tokens (the navigator's), the system's
//! light or dark from `prefers-color-scheme`. What `apps/ui/src/theme.rs`
//! does on a desk, without its themes folder (a browser has none).

use architect_theme::{Scheme, ThemeHandle, ThemeStore};
use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

/// The app's own layer: the navigator's tokens.
fn app_layer() -> architect_theme::Layer {
    architect_navigator::ui::with_tokens(architect_theme::Layer::default())
}

/// The app's theme store, in context, its actions on `list`.
pub fn use_web_theme(list: &architect::action::ActionList) -> ThemeHandle {
    let handle = architect_theme::use_theme_store(ThemeStore::default);
    let store = handle.store();
    use_hook(|| {
        let _ = architect_theme::actions::register(list, &store);
        let _ = store.set(architect_theme::LayerKind::App, app_layer());
    });
    handle
}

fn dark() -> Option<web_sys::MediaQueryList> {
    web_sys::window()?
        .match_media("(prefers-color-scheme: dark)")
        .ok()
        .flatten()
}

/// Follows the system's light or dark, as it changes.
#[component]
pub fn SystemScheme() -> Element {
    let handle = architect_theme::use_theme();
    use_hook(move || {
        let Some(query) = dark() else { return };
        let scheme = |dark: bool| if dark { Scheme::Dark } else { Scheme::Light };
        let _ = handle.set_system(scheme(query.matches()));
        let on_change = Closure::<dyn Fn(web_sys::Event)>::new(move |_| {
            if let Some(q) = dark() {
                let _ = handle.set_system(scheme(q.matches()));
            }
        });
        let _ =
            query.add_event_listener_with_callback("change", on_change.as_ref().unchecked_ref());
        on_change.forget();
    });
    rsx! {}
}

/// Where the app's code and face are (`<html data-assets>`, which
/// `start.js` sets: `/assets/app-<sha12>/`, cached for good;
/// `scripts/web-bundle.sh`). `None` where the page names none.
#[must_use]
pub fn assets() -> Option<String> {
    let dir = web_sys::window()?
        .document()?
        .document_element()?
        .get_attribute("data-assets")?;
    // Only the bundle's own folder on this origin.
    (dir.starts_with("/assets/app-")
        && dir.ends_with('/')
        && dir
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '-')))
    .then_some(dir)
}

/// The page under the app: its background the theme's desk, the press
/// look of the app's rows (`apps/ui/src/theme.rs` `page_css`), and the
/// app's face (Inter, from `assets`, its folder; without one the system's
/// face).
#[must_use]
pub fn page_css(resolved: &architect_theme::Resolved, assets: Option<&str>) -> String {
    let face = assets.map_or_else(String::new, |dir| {
        format!(
            "@font-face {{ font-family: 'Inter'; src: url('{dir}Inter-Variable.woff2') format('woff2'); \
             font-weight: 100 900; font-display: swap; }}"
        )
    });
    format!(
        "{face}html, body {{ margin: 0; padding: 0; background: {desk}; }}\
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
