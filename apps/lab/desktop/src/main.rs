//! The app shell's lab in a desk window, painted by Blitz as the apps
//! are. `ARCHITECT_DEVICE=iphone|iphone-side|ipad|ipad-side` opens it
//! standing in for that device (its size, touch and insets, with its
//! status bar and housing drawn over the page); the ⋯ menu changes it
//! live. Served with `dx serve`, an `rsx!` edit in the lab or in
//! `architect-shell` shows at once. Launched by the workspace's
//! launcher, so a Vault tab dragged out of the window opens a window of
//! its own.

use architect_lab_ui::{Device, LabApp, use_window_screen};
use dioxus::prelude::*;
use dioxus_native::WindowAttributes;
use dioxus_native::winit::dpi::LogicalSize;

fn first_device() -> Device {
    Device::named(&std::env::var("ARCHITECT_DEVICE").unwrap_or_default())
}

#[component]
fn App() -> Element {
    let _screen = use_window_screen(first_device());
    rsx! { LabApp {} }
}

fn main() {
    let (w, h) = first_device().size().unwrap_or((1280.0, 820.0));
    let window = WindowAttributes::default()
        .with_title("Architect — app shell lab")
        .with_surface_size(LogicalSize::new(w, h));
    // The shell draws the title bar: the content runs under a transparent
    // one, the traffic lights placed in the shell's 40pt row
    // (`use_window_chrome`).
    #[cfg(target_os = "macos")]
    let window = window.with_platform_attributes(Box::new(
        dioxus_native::winit::platform::macos::WindowAttributesMacOS::default()
            .with_titlebar_transparent(true)
            .with_fullsize_content_view(true)
            .with_title_hidden(true),
    ));
    architect_workspace::native::launch_with_fonts(App, window, architect_lab_ui::FONTS);
}
