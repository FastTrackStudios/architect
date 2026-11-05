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
//! architect-desktop mcp --app <socket>         # MCP over another running lab (the iOS lab)
//! architect-desktop actions shot /tmp/lab.png  # the window, drawn by the lab (APP_SHOT)
//! architect-desktop extensions check <folder>  # EXT_CHECK with no window; nonzero when it fails
//! ```
//!
//! In a window it keeps `<app dir>/AGENTS.md` and `actions.schema.json`
//! written from its live list (`architect_control::guide`).

mod mcp;

use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use architect::action::ActionList;
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
    // The lab's Action List, made here so MCP and the window's control
    // socket serve the one its components fill (`LabApp` takes it from
    // context).
    let list = use_context_provider(|| Arc::new(ActionList::new()));
    use_hook(|| mcp::serve(&list));
    use_hook(|| Rc::new(architect_desktop::serve_window(&list)));
    // `APP_SHOT`: the window draws itself to a PNG.
    architect_workspace::shot::use_app_shot(&list);
    // `<app dir>/AGENTS.md`: the agents' guide, from the live list.
    use_hook(|| Rc::new(architect_desktop::keep_guide_written(&list)));
    rsx! { LabApp {} }
}

fn main() -> ExitCode {
    let first = std::env::args().nth(1);
    match first.as_deref() {
        Some("actions") => return architect_desktop::cli().main(std::env::args_os().skip(1)),
        Some("mcp") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            let app = args
                .iter()
                .skip_while(|a| *a != "--app")
                .nth(1)
                .map(std::path::PathBuf::from);
            return architect_desktop::mcp(args.iter().any(|a| a == "--headless"), app);
        }
        Some("--headless") => return architect_desktop::serve_headless(),
        Some("extensions") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return architect_desktop::extensions(&rest);
        }
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
    // Off a Mac the shell draws the whole title bar, its window buttons
    // too (Obsidian's): no system decorations, so a compositor that draws
    // server-side ones (Loft's niri, KWin) adds no strip above the window.
    #[cfg(not(target_os = "macos"))]
    let window = window.with_decorations(false);
    architect_workspace::native::launch_with_fonts(App, window, architect_lab_ui::FONTS);
    ExitCode::SUCCESS
}
