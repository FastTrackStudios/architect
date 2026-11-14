//! The window, and the app's Action List from the command line and over
//! MCP (feature `gui`). See the crate docs.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use architect::action::{ActionList, Principal};
use architect_app::{Device, LabApp, use_window_screen};
use architect_control::Surface;
use architect_control::cli::{Cli, Headless};
use architect_control::server::{Control, Serving};
use architect_distribution::Distribution;
use architect_guide::ViewSize;
use dioxus_native::WindowAttributes;
use dioxus_native::winit::dpi::LogicalSize;

use architect_shell::{Insets, Screen};
use dioxus::prelude::*;

use crate::mcp_port;

/// The app's name to `--app`, its socket's and its guide's: its
/// distribution's (`architect-desktop` for the Architect app,
/// `architect-sandbox` for the sandbox, each binary's own name).
#[must_use]
pub fn app() -> &'static str {
    architect_app::distribution().app_name()
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

const EXTENSIONS_USAGE: &str = "usage: architect-desktop extensions check <folder>\n\
       architect-desktop extensions dev <folder> [--app <socket>]      # EXT_DEV in the running app\n\
       architect-desktop extensions undev <folder> [--app <socket>]    # EXT_DEV_END\n\
       architect-desktop extensions registries [--app <socket>]        # EXT_REGISTRIES";

/// `architect-desktop extensions …`: check a folder, or load it into the
/// running app.
///
/// `check <folder>`, the extension checked as the lab would load it, its report printed as JSON (nonzero
/// when it does not pass); `dev <folder>` / `undev <folder>`, a package
/// folder loaded into (or taken out of) the running app as a development
/// copy (`EXT_DEV`, `EXT_DEV_END`, over its control socket); `registries`,
/// `EXT_REGISTRIES`.
#[must_use]
pub fn extensions(args: &[String]) -> ExitCode {
    let usage = || {
        let _ = writeln!(std::io::stderr(), "{EXTENSIONS_USAGE}");
        ExitCode::from(2)
    };
    let Some((verb, rest)) = args.split_first() else {
        return usage();
    };
    // `--app <socket>` and the like go to the actions CLI as they are.
    let (folder, flags) = match rest.split_first() {
        Some((f, flags)) if !f.starts_with("--") => (Some(f.as_str()), flags),
        _ => (None, rest),
    };
    let on_app = |id: &str, pair: Option<String>| {
        let mut command: Vec<String> = vec!["actions".into(), "run".into(), id.into()];
        if let Some(pair) = pair {
            command.extend(["--arg".to_owned(), pair]);
        }
        command.extend(flags.iter().cloned());
        cli().main(command)
    };
    let absolute = |f: &str| {
        std::fs::canonicalize(f).map_or_else(|_| f.to_owned(), |p| p.display().to_string())
    };
    match (verb.as_str(), folder) {
        ("check", Some(folder)) if flags.is_empty() => check_extension(folder),
        ("dev", Some(folder)) => on_app("EXT_DEV", Some(format!("folder={}", absolute(folder)))),
        ("undev", Some(folder)) => {
            on_app("EXT_DEV_END", Some(format!("folder={}", absolute(folder))))
        }
        ("registries", None) => on_app("EXT_REGISTRIES", None),
        _ => usage(),
    }
}

fn check_extension(folder: &str) -> ExitCode {
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

/// `ARCHITECT_WEBVIEW_X11=1` on Linux: the window opens under `XWayland`
/// (`WAYLAND_DISPLAY` is dropped while `DISPLAY` is there), the one way a
/// web view pane can be a child of it today (#222: `WebKitGTK` embeds a child
/// only in an X11 window; `docs/content/architecture/embedded-views.md`,
/// "Web views").
fn xwayland_for_web_views() {
    #[cfg(target_os = "linux")]
    if std::env::var_os("ARCHITECT_WEBVIEW_X11").is_some_and(|v| v != "0")
        && std::env::var_os("DISPLAY").is_some()
        && std::env::var_os("WAYLAND_DISPLAY").is_some()
    {
        // SAFETY: the first thing the program does, before any thread
        // that could read the environment starts.
        unsafe { std::env::remove_var("WAYLAND_DISPLAY") };
    }
}

/// The whole of a desk binary's `main`: the app `distribution` is.
///
/// The Action List's CLI, MCP, the headless app, the node (`serve`, with
/// the distribution's block renderers), `distribution` (what it is, as
/// JSON) or the window. A distribution that adopts an older folder (the
/// Architect app, the lab's) copies it into its own before its window
/// opens (`Distribution::adopt_folder`).
#[must_use]
pub fn run(distribution: Distribution) -> ExitCode {
    xwayland_for_web_views();
    let dist = distribution.install();
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
        Some("serve") => return crate::node::main(std::env::args_os().skip(2), dist),
        Some("files") => return crate::node::cli::files(std::env::args_os().skip(2)),
        Some("agent") => return crate::node::cli::agent(std::env::args_os().skip(2)),
        Some(crate::node::cli::GUARD) => {
            return crate::node::cli::guard(std::env::args_os().skip(2));
        }
        Some("extensions") => {
            let rest: Vec<String> = std::env::args().skip(2).collect();
            return extensions(&rest);
        }
        Some("links") => return links(dist.name(), std::env::args().nth(2).as_deref()),
        Some("distribution") => {
            return crate::distribution_command(dist, std::env::args().nth(2).as_deref());
        }
        _ => {}
    }
    if architect_presence::single_instance(&dist.instance_id())
        == architect_presence::Launch::HandedOff
    {
        return ExitCode::SUCCESS;
    }
    let _ = dist.adopt_folder();
    let (w, h) = first_device().size().unwrap_or((1280.0, 820.0));
    let window = WindowAttributes::default()
        .with_title(dist.name())
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

/// `<app> links register`: this binary opens the app's links
/// (`architect://open?url=…`, #629). An installed bundle declares them
/// itself (the `AppImage`'s desktop entry, the Mac's `Info.plist`).
fn links(name: &str, verb: Option<&str>) -> ExitCode {
    if verb != Some("register") {
        eprintln!("usage: <app> links register");
        return ExitCode::FAILURE;
    }
    if !cfg!(target_os = "linux") {
        eprintln!("links: an installed app declares its links itself; nothing to do here");
        return ExitCode::SUCCESS;
    }
    let (Ok(exe), Some((data, config))) = (std::env::current_exe(), crate::links::xdg_homes())
    else {
        eprintln!("links: no executable path or home folder");
        return ExitCode::FAILURE;
    };
    match crate::links::register(name, architect_app::presence::id(), &exe, &data, &config) {
        Ok(done) => {
            // Best effort: desktops that cache the entries read them again.
            let _ = std::process::Command::new("update-desktop-database")
                .arg(data.join("applications"))
                .status();
            println!(
                "{} opens {}:// links: {} (default in {})",
                exe.display(),
                architect_spaces::guest::APP_SCHEME,
                done.entry.display(),
                done.mimeapps.display()
            );
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("links: {why}");
            ExitCode::FAILURE
        }
    }
}
