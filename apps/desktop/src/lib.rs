//! The Architect app on a desk (Blitz), and its Action List from the
//! command line and over MCP, windowed or headless: [`run`] is the whole
//! of `architect-desktop`'s `main` (the Architect app) and of
//! `architect-sandbox`'s (`apps/sandbox`, the dev sandbox), each with its
//! [`Mode`].
//!
//! - In the window, the app serves its control socket
//!   (`architect-control`; on in a debug build, `FTS_CONTROL=0|1` says
//!   otherwise): `architect-desktop actions list` asks it.
//! - `architect-desktop --headless` is the app with no window, serving the
//!   same socket until it is stopped: its root mounted in a headless
//!   document (`architect_guide::run::Headless`), so every action its
//!   components register is there.
//! - `architect-desktop actions … --headless` starts that for one command.
//! - `architect-desktop mcp [--headless]` is MCP on stdio for a model:
//!   bridged onto the running lab's socket, or over a headless one.

mod mcp_port;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use architect::action::{ActionList, Principal};
use architect_app::{Device, LabApp, Mode, use_window_screen};
use architect_control::Surface;
use architect_control::cli::{Cli, Headless};
use architect_control::server::{Control, Serving};
use architect_guide::ViewSize;
use dioxus_native::WindowAttributes;
use dioxus_native::winit::dpi::LogicalSize;

use architect_shell::{Insets, Screen};
use dioxus::prelude::*;

/// The app's name to `--app`, its socket's and its guide's:
/// `architect-desktop` for the Architect app, `architect-sandbox` for the
/// sandbox (each binary's own name).
#[must_use]
pub fn app() -> &'static str {
    match Mode::current() {
        Mode::Architect => "architect-desktop",
        Mode::Sandbox => "architect-sandbox",
    }
}

/// The lab's list as the CLI and MCP ask it, as `service:cli`.
///
/// A caller from outside, so the lab asks a person before it deletes (the
/// confirmation sheet, `apps/ui/src/confirm.rs`), as it asks before a
/// model over MCP does.
#[must_use]
pub fn surface(list: &ActionList) -> Surface {
    Surface::new(app(), env!("CARGO_PKG_VERSION"), list.clone()).with_principal(
        Principal::Service {
            name: "cli".to_owned(),
        },
    )
}

/// The window's control socket, when it is wanted
/// (`architect_control::endpoint::wanted`). `None` too when it could not be
/// bound: the lab is one instance, so a socket in use is another lab's.
#[must_use]
pub fn serve_window(list: &ActionList) -> Option<Serving> {
    Control::new(app(), surface(list))
        .ok()?
        .serve_if_wanted()
        .ok()
        .flatten()
}

/// The lab's opening of its agents' guide (`architect_control::guide`).
pub const GUIDE_PREAMBLE: &str = include_str!("AGENTS.preamble.md");

/// The lab's agents' guide, for `list`.
#[must_use]
pub fn guide() -> architect_control::guide::Guide {
    architect_app::page_types().into_iter().fold(
        architect_control::guide::Guide::new(app()).preamble(GUIDE_PREAMBLE),
        |guide, (name, about, text)| guide.file(name, about, text),
    )
}

/// Keeps the agents' guide written from `list` while the window runs.
///
/// `<app dir>/AGENTS.md`, `actions.schema.json` and the page types
/// (`architect.d.ts`, `architect.d.luau`): once it has settled, and again
/// whenever an action comes or goes. `None` with no app folder.
#[must_use]
pub fn keep_guide_written(list: &ActionList) -> Option<architect_control::guide::KeptWritten> {
    let dir = architect_app::app_dir()?;
    Some(guide().keep_written(surface(list), dir, Duration::from_millis(1500)))
}

/// `architect-desktop extensions check <folder>`: the extension checked
/// as the lab would load it, its report printed as JSON; nonzero when it
/// does not pass.
#[must_use]
pub fn extensions(args: &[String]) -> ExitCode {
    let [verb, folder] = args else {
        let _ = writeln!(
            std::io::stderr(),
            "usage: architect-desktop extensions check <folder>"
        );
        return ExitCode::from(2);
    };
    if verb != "check" {
        let _ = writeln!(
            std::io::stderr(),
            "usage: architect-desktop extensions check <folder>"
        );
        return ExitCode::from(2);
    }
    let report =
        architect_app::extension_checker().check(&architect_extensions::check::folder(folder));
    let json = facet_json::to_string(&report).unwrap_or_default();
    let _ = writeln!(std::io::stdout(), "{json}");
    if report.ok {
        ExitCode::SUCCESS
    } else {
        let _ = writeln!(std::io::stderr(), "architect-desktop: {}", report.summary());
        ExitCode::from(1)
    }
}

/// The lab with no window: its root in a headless document, its list.
pub struct HeadlessLab {
    app: architect_guide::run::Headless,
    surface: Surface,
}

impl HeadlessLab {
    /// The lab mounted as its desk window mounts it, at a desk's size.
    #[must_use]
    pub fn open() -> Self {
        let list = Arc::new(ActionList::new());
        let app =
            architect_guide::run::App::new(Root, list.clone()).fonts(architect_app::font_context());
        Self {
            app: architect_guide::run::Headless::open(&app, ViewSize::default()),
            surface: surface(&list),
        }
    }
}

impl Headless for HeadlessLab {
    fn surface(&self) -> Surface {
        self.surface.clone()
    }

    fn pump(&mut self) -> bool {
        self.app.pump()
    }
}

/// What the lab's desk window gives it, headless: a desk screen of the
/// document's size.
#[component]
fn Root() -> Element {
    let view = use_context::<ViewSize>();
    use_context_provider(|| Signal::new(Device::Native));
    use_context_provider(|| {
        Signal::new(Screen {
            width: f64::from(view.width),
            height: f64::from(view.height),
            touch: false,
            insets: Insets::default(),
        })
    });
    rsx! { LabApp {} }
}

/// `architect-desktop actions …`.
#[must_use]
pub fn cli() -> Cli {
    Cli::new(app()).headless(|| Ok(Box::new(HeadlessLab::open())))
}

/// How long a pump loop sleeps when the lab had nothing to do.
const IDLE: Duration = Duration::from_millis(5);

/// `architect-desktop --headless`: the lab with no window, serving its
/// socket until the process is stopped. Prints the socket's path.
#[must_use]
pub fn serve_headless() -> ExitCode {
    let mut lab = HeadlessLab::open();
    lab.settle();
    let serving = match Control::new(app(), lab.surface()).and_then(Control::serve) {
        Ok(serving) => serving,
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "architect-desktop: {e}");
            return ExitCode::from(69);
        }
    };
    let _ = writeln!(std::io::stdout(), "{}", serving.path().display());
    loop {
        if !lab.pump() {
            std::thread::sleep(IDLE);
        }
    }
}

/// `architect-desktop mcp [--headless | --app <socket>]`: MCP on stdio.
///
/// Over the running lab's socket, over another running lab's (`--app`:
/// the iOS lab's, which has no stdio of its own;
/// `scripts/mac-live-check.sh`), or over a headless lab pumped here.
#[must_use]
pub fn mcp(headless: bool, socket: Option<PathBuf>) -> ExitCode {
    let failed = |e: &dyn std::fmt::Display| {
        let _ = writeln!(std::io::stderr(), "architect-desktop mcp: {e}");
        ExitCode::from(69)
    };
    if !headless {
        let path: PathBuf =
            match socket.map_or_else(|| architect_control::endpoint::socket_path(app()), Ok) {
                Ok(path) => path,
                Err(e) => return failed(&e),
            };
        return match architect_mcp::bridge::serve_stdio_via(&path) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => failed(&e),
        };
    }
    let mut lab = HeadlessLab::open();
    lab.settle();
    let server = architect_mcp::Server::over(lab.surface());
    let stdio = std::thread::spawn(move || architect_mcp::serve_stdio(&server));
    while !stdio.is_finished() {
        if !lab.pump() {
            std::thread::sleep(IDLE);
        }
    }
    match stdio.join() {
        Ok(Ok(())) => ExitCode::SUCCESS,
        Ok(Err(e)) => failed(&e),
        Err(_) => failed(&"the stdio thread failed"),
    }
}

/// The device the window opens standing in for (`ARCHITECT_DEVICE`).
fn first_device() -> Device {
    Device::named(&std::env::var("ARCHITECT_DEVICE").unwrap_or_default())
}

/// The window's root.
#[component]
fn Window() -> Element {
    let _screen = use_window_screen(first_device());
    // The app's Action List, made here so MCP and the window's control
    // socket serve the one its components fill (`LabApp` takes it from
    // context).
    let list = use_context_provider(|| Arc::new(ActionList::new()));
    use_hook(|| mcp_port::serve(&list));
    use_hook(|| Rc::new(serve_window(&list)));
    // `APP_SHOT`: the window draws itself to a PNG.
    architect_workspace::shot::use_app_shot(&list);
    // `<app dir>/AGENTS.md`: the agents' guide, from the live list.
    use_hook(|| Rc::new(keep_guide_written(&list)));
    rsx! { LabApp {} }
}

/// The whole of a desk binary's `main`, as `mode`.
///
/// The Action List's CLI, MCP, the headless app or the window. The
/// Architect app copies the lab's old folder into its own before its
/// window opens (`architect_app::mode::migrate_lab_folder`).
#[must_use]
pub fn run(mode: Mode) -> ExitCode {
    mode.set();
    let first = std::env::args().nth(1);
    match first.as_deref() {
        Some("actions") => return cli().main(std::env::args_os().skip(1)),
        Some("mcp") => {
            let args: Vec<String> = std::env::args().skip(2).collect();
            let socket = args
                .iter()
                .skip_while(|a| *a != "--app")
                .nth(1)
                .map(PathBuf::from);
            return mcp(args.iter().any(|a| a == "--headless"), socket);
        }
        Some("--headless") => return serve_headless(),
        Some("extensions") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return extensions(&rest);
        }
        _ => {}
    }
    if architect_presence::single_instance(architect_app::presence::id())
        == architect_presence::Launch::HandedOff
    {
        return ExitCode::SUCCESS;
    }
    if mode == Mode::Architect {
        let _ = architect_app::mode::migrate_lab_folder();
    }
    let (w, h) = first_device().size().unwrap_or((1280.0, 820.0));
    let window = WindowAttributes::default()
        .with_title(mode.name())
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
    architect_workspace::native::launch_with_fonts(Window, window, architect_app::FONTS);
    ExitCode::SUCCESS
}
