//! The app shell's lab on an iPhone or an iPad, painted by Blitz into
//! winit's `UIKit` window. `dx serve --ios` (README.md) puts it on the
//! simulator with hot reload: an `rsx!` edit in the lab or in
//! `architect-shell` shows on the phone at once.
//!
//! Off iOS it runs as a plain window, so `cargo check` holds everywhere.

#[cfg(target_os = "ios")]
mod scene;

use architect_lab_ui::{Device, LabApp, use_window_screen};
use dioxus::prelude::*;
use dioxus_native::{Config, WindowAttributes, launch_cfg};

#[component]
fn App() -> Element {
    // winit's window joins the app's scene, without which iOS 27 never
    // shows it.
    #[cfg(target_os = "ios")]
    {
        let window = dioxus_native::use_window();
        use_hook(move || {
            use dioxus_native::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            if let Ok(handle) = window.window_handle()
                && let RawWindowHandle::UiKit(uikit) = handle.as_raw()
            {
                scene::window_created(uikit.ui_view);
            }
        });
    }
    let _screen = use_window_screen(Device::Native);
    rsx! { LabApp {} }
}

fn main() {
    // The page is the whole screen, under the status bar and the home
    // indicator: the shell keeps its bars clear of them itself, from the
    // safe area it is handed (`use_window_screen`), as an iOS app does.
    // SAFETY: first thing in main, before any thread starts.
    unsafe { std::env::set_var("BLITZ_SAFE_AREA", "0") };
    #[cfg(target_os = "ios")]
    scene::register();
    // The keyboard's height from UIKit, for the bar that rides it.
    architect_shell::install_keyboard();
    let window = WindowAttributes::default().with_title("Architect");
    launch_cfg(
        App,
        vec![],
        vec![Box::new(
            Config::new()
                .with_window_attributes(window)
                .with_font_ctx(architect_lab_ui::font_context()),
        )],
    );
}
