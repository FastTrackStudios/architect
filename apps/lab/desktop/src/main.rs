//! The app shell's lab in a desk window, painted by Blitz as the apps
//! are. `ARCHITECT_DEVICE=iphone|iphone-side|ipad|ipad-side` opens it
//! standing in for that device (its size, touch and insets, with its
//! status bar and housing drawn over the page); the lab's menu (the flask) changes
//! it live. Served with `dx serve`, an `rsx!` edit in the lab or in
//! `architect-shell` shows at once. Launched by the workspace's
//! launcher, so a Vault tab dragged out of the window opens a window of
//! its own.
//!
//! With `ARCHITECT_LAB_MCP_PORT` set it serves its Action List over MCP on
//! that loopback port (`mcp`).
//!
//! One instance: a second launch brings the first forward and ends. With
//! `--background` (a login launch) it starts in the menu bar, with no
//! window and no Dock icon.
//!
//! Its Action List from the command line and over MCP (`src/lib.rs`):
//!
//! ```text
//! architect-desktop actions list               # ask the running lab
//! architect-desktop actions run LAB_STATE --headless
//! architect-desktop --headless                 # the lab with no window, serving its socket
//! architect-desktop mcp [--headless]           # MCP on stdio
//! ```

mod mcp;

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use architect::action::ActionList;
use architect_a2ui::Surfaces;
use architect_lab_ui::{Device, LabApp, use_window_screen};
use dioxus::prelude::*;
use dioxus_native::WindowAttributes;
use dioxus_native::winit::dpi::LogicalSize;

fn first_device() -> Device {
    Device::named(&std::env::var("ARCHITECT_DEVICE").unwrap_or_default())
}

/// The lab's agents' surfaces in their file
/// ([`architect_lab_ui::surfaces_file`]), so the "Agent UI" shows them
/// again after a restart. Opened once, for every window: one writer to the
/// file. `None` when it cannot be: the lab goes on with surfaces kept
/// nowhere, and says why in the log.
fn surfaces() -> Option<Surfaces> {
    /// The surfaces, and the runtime their file is written on for the
    /// app's life (the window's own runs no tokio).
    static OPENED: OnceLock<Option<(Surfaces, tokio::runtime::Runtime)>> = OnceLock::new();
    let open = || {
        let path = architect_lab_ui::surfaces_file()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok()?;
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("lab-surfaces")
            .enable_all()
            .build()
            .inspect_err(|error| tracing::error!(%error, "the surfaces' runtime did not start"))
            .ok()?;
        let surfaces = runtime
            .block_on(Surfaces::open_file(&path))
            .inspect_err(|error| {
                tracing::error!(%error, path = %path.display(), "the lab's surfaces are kept nowhere");
            })
            .ok()?;
        Some((surfaces, runtime))
    };
    OPENED
        .get_or_init(open)
        .as_ref()
        .map(|(surfaces, _)| surfaces.clone())
}

#[component]
fn App() -> Element {
    let _screen = use_window_screen(first_device());
    // The lab's Action List, made here so MCP and the window's control
    // socket serve the one its components fill (`LabApp` takes it from
    // context).
    let list = use_context_provider(|| Arc::new(ActionList::new()));
    // An agent's surfaces, kept on this device (`LabApp` takes them from
    // context too).
    use_hook(|| {
        if let Some(surfaces) = surfaces() {
            provide_context(surfaces);
        }
    });
    use_hook(|| mcp::serve(&list));
    use_hook(|| Rc::new(architect_desktop::serve_window(&list)));
    rsx! { LabApp {} }
}

fn main() -> ExitCode {
    let first = std::env::args().nth(1);
    match first.as_deref() {
        Some("actions") => return architect_desktop::cli().main(std::env::args_os().skip(1)),
        Some("mcp") => {
            return architect_desktop::mcp(std::env::args().any(|a| a == "--headless"));
        }
        Some("--headless") => return architect_desktop::serve_headless(),
        _ => {}
    }
    if architect_presence::single_instance(architect_lab_ui::presence::ID)
        == architect_presence::Launch::HandedOff
    {
        return ExitCode::SUCCESS;
    }
    let (w, h) = first_device().size().unwrap_or((1280.0, 820.0));
    let window = WindowAttributes::default()
        .with_title("Architect — app shell lab")
        .with_surface_size(LogicalSize::new(w, h))
        .with_visible(!architect_presence::launched_in_background());
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
    ExitCode::SUCCESS
}
