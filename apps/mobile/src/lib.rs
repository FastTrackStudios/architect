//! The Architect app on an iPhone or an iPad, painted by Blitz into
//! winit's `UIKit` window: [`run`] is the whole of `architect-mobile`'s
//! `main` (the Architect app, or with `ARCHITECT_SANDBOX=1` the dev
//! sandbox) and of any build of someone's own for iOS, each with its
//! [`Distribution`] (#491, #640). It is `architect_desktop::run`'s
//! counterpart on a phone: a distribution crate calls one or the other,
//! by target, with the same `Distribution`, so the extensions it links
//! are in the iOS app too (`dist/standard`, `cargo xtask dist ios`).
//!
//! `dx serve --ios` (`apps/sandbox/README.md`) puts the app on the
//! simulator with hot reload: an `rsx!` edit in `apps/ui` or in
//! `architect-shell` shows on the phone at once. Off iOS it runs as a
//! plain window, so `cargo check` holds everywhere.
//!
//! iOS allows no JIT and no downloaded native code (App Store rules 2.5.2
//! and 4.7), so what a distribution runs on a phone is what it links in:
//! its linked extensions, and WASI components on Pulley with `wasi`.

#[cfg(target_os = "ios")]
mod scene;

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::{ActionList, Principal};
use architect_app::{Device, LabApp, use_window_screen};
use architect_control::Surface;
use architect_control::server::Control;
pub use architect_distribution::{self as distribution, Bundle, Distribution, Icon};
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
    // The lab's Action List on its control socket, as the desk lab's
    // (on in a debug build, `FTS_CONTROL=0|1` says otherwise): in the
    // simulator, with `FTS_CONTROL_DIR=<a short folder on the Mac>`,
    // `architect-desktop actions --app <it>/architect-mobile.sock run …`
    // drives the phone (`scripts/mac-live-check.sh ios-act`).
    let list = use_context_provider(|| Arc::new(ActionList::new()));
    use_hook(|| Rc::new(serve_control(&list)));
    rsx! { LabApp {} }
}

/// The control socket, when it is wanted and could be bound.
///
/// Its MCP face is the desk lab's bridge onto this socket
/// (`architect-desktop mcp --app <it>/architect-mobile.sock`): an iOS app
/// has no stdio to serve MCP on. The list is the lab's own
/// (`architect-app`), whose parity the desk lab's test checks.
// surfaces: mcp via apps/desktop
fn serve_control(list: &ActionList) -> Option<architect_control::server::Serving> {
    let surface = Surface::new("architect-mobile", env!("CARGO_PKG_VERSION"), list.clone())
        .with_principal(Principal::Service {
            name: "cli".to_owned(),
        });
    Control::new("architect-mobile", surface)
        .ok()?
        .serve_if_wanted()
        .ok()
        .flatten()
}

/// The whole of an iOS app's `main`.
///
/// `distribution` is installed (its name, folder and linked extensions,
/// as `architect_desktop::run` installs it), the `UIScene` delegate
/// registered, and the app launched into winit's window. On iOS it never
/// returns (`UIApplicationMain`).
///
/// Call it first thing in `main`, before any thread starts: it sets the
/// process's environment (`BLITZ_SAFE_AREA`).
///
/// ```no_run
/// fn main() -> std::process::ExitCode {
///     architect_mobile::run(architect_mobile::distribution::architect())
/// }
/// ```
#[must_use]
pub fn run(distribution: Distribution) -> ExitCode {
    // The page is the whole screen, under the status bar and the home
    // indicator: the shell keeps its bars clear of them itself, from the
    // safe area it is handed (`use_window_screen`), as an iOS app does.
    // SAFETY: `run` is the first thing in `main`, before any thread
    // starts (its contract above).
    unsafe { std::env::set_var("BLITZ_SAFE_AREA", "0") };
    let dist = distribution.install();
    let _ = dist.adopt_folder();
    #[cfg(target_os = "ios")]
    scene::register();
    // The keyboard's height from UIKit, for the bar that rides it.
    architect_shell::install_keyboard();
    let window = WindowAttributes::default().with_title(dist.name());
    launch_cfg(
        App,
        vec![],
        vec![Box::new(
            Config::new()
                .with_window_attributes(window)
                .with_font_ctx(architect_app::font_context()),
        )],
    );
    ExitCode::SUCCESS
}
