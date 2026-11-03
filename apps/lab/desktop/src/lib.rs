//! The lab's Action List from the command line and over MCP, windowed or
//! headless — what `src/main.rs` runs before (or instead of) a window.
//!
//! - In the window, the lab serves its control socket
//!   (`architect-control`; on in a debug build, `FTS_CONTROL=0|1` says
//!   otherwise): `architect-desktop actions list` asks it.
//! - `architect-desktop --headless` is the lab with no window, serving the
//!   same socket until it is stopped: its root mounted in a headless
//!   document (`architect_guide::run::Headless`), so every action its
//!   components register is there.
//! - `architect-desktop actions … --headless` starts that for one command.
//! - `architect-desktop mcp [--headless]` is MCP on stdio for a model:
//!   bridged onto the running lab's socket, or over a headless one.

use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use architect::action::{ActionList, Principal};
use architect_control::Surface;
use architect_control::cli::{Cli, Headless};
use architect_control::server::{Control, Serving};
use architect_guide::ViewSize;
use architect_lab_ui::{Device, LabApp};
use architect_shell::{Insets, Screen};
use dioxus::prelude::*;

/// The lab's name to `--app`, its socket's name.
pub const APP: &str = "architect-desktop";

/// The lab's list as the CLI and MCP ask it, as `service:cli`.
///
/// A caller from outside, so the lab asks a person before it deletes (the
/// confirmation sheet, `apps/lab/ui/src/confirm.rs`), as it asks before a
/// model over MCP does.
#[must_use]
pub fn surface(list: &ActionList) -> Surface {
    Surface::new(APP, env!("CARGO_PKG_VERSION"), list.clone()).with_principal(Principal::Service {
        name: "cli".to_owned(),
    })
}

/// The window's control socket, when it is wanted
/// (`architect_control::endpoint::wanted`). `None` too when it could not be
/// bound: the lab is one instance, so a socket in use is another lab's.
#[must_use]
pub fn serve_window(list: &ActionList) -> Option<Serving> {
    Control::new(APP, surface(list))
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
    architect_control::guide::Guide::new(APP).preamble(GUIDE_PREAMBLE)
}

/// Keeps `<app dir>/AGENTS.md` and `actions.schema.json` written from
/// `list` while the window runs: once it has settled, and again whenever
/// an action comes or goes. `None` with no app folder.
#[must_use]
pub fn keep_guide_written(list: &ActionList) -> Option<architect_control::guide::KeptWritten> {
    let dir = architect_lab_ui::app_dir()?;
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
        architect_lab_ui::extension_checker().check(&architect_extensions::check::folder(folder));
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
        let app = architect_guide::run::App::new(Root, list.clone())
            .fonts(architect_lab_ui::font_context());
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
    Cli::new(APP).headless(|| Ok(Box::new(HeadlessLab::open())))
}

/// How long a pump loop sleeps when the lab had nothing to do.
const IDLE: Duration = Duration::from_millis(5);

/// `architect-desktop --headless`: the lab with no window, serving its
/// socket until the process is stopped. Prints the socket's path.
#[must_use]
pub fn serve_headless() -> ExitCode {
    let mut lab = HeadlessLab::open();
    lab.settle();
    let serving = match Control::new(APP, lab.surface()).and_then(Control::serve) {
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

/// `architect-desktop mcp [--headless]`: MCP on stdio, over the running
/// lab's socket or over a headless lab pumped here.
#[must_use]
pub fn mcp(headless: bool) -> ExitCode {
    let failed = |e: &dyn std::fmt::Display| {
        let _ = writeln!(std::io::stderr(), "architect-desktop mcp: {e}");
        ExitCode::from(69)
    };
    if !headless {
        let path: PathBuf = match architect_control::endpoint::socket_path(APP) {
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
