//! Extensions' `process` views (feature `extension`, #838): any
//! extension's `[[views]] kind = "process"`, drawn by [`ProcessViewPane`]
//! whichever copy of it is loaded (built in, installed or in
//! development).
//!
//! [`use_process_views`] provides `architect_extensions::interface::ProcessViews`
//! for the app's window: `ExtensionView` asks it for each such view. Each
//! pane:
//!
//! - checks the view with `architect_extensions::process_view::launch`
//!   (the platform, the grant, the program a file in the package) and
//!   says why in its place when it cannot run;
//! - starts the program when it opens ([`open`]), one session per pane
//!   (`<extension>/<view>#<pane>`), so a pane moved to another tab group
//!   or a pop-out window keeps its program;
//! - starts it again when its stamp changes (a development copy rebuilt,
//!   which `EXT_DEV` reloads; another copy loaded), and when it dies, as a
//!   process extension's program is ([`Restart::default`]), saying why;
//! - stops it when its pane has been gone for [`PARKED`] (closed, not
//!   moved), at once when its extension unloads or its grant is taken
//!   back ([`reap`]).

use std::time::Duration;

use architect_extensions::host::Host;
use architect_extensions::interface::{ProcessAsk, ProcessViews};
use architect_extensions::process_view::{Launch, launch};
use std::rc::Rc;
use std::sync::Arc;

use architect_workspace::use_pane;
use dioxus::prelude::*;

use crate::host::{self, ProcessViewPane, Program, Restart, Session};

/// What forces a program's frames over shared memory (`shm`), as for the
/// lab's (`guest::TRANSPORT_ENV`).
pub const TRANSPORT_ENV: &str = "ARCHITECT_PROCESS_VIEW_TRANSPORT";

/// The prefix of extensions' sessions' keys.
pub const PREFIX: &str = "ext:";

/// How long a session whose pane went away is kept for it to come back
/// (a tab switch, a move to another group or window).
pub const PARKED: Duration = Duration::from_secs(10);

/// How often sessions are checked against the host.
pub const REAP: Duration = Duration::from_secs(1);

/// The session key of extension `extension`'s view `view` in pane `pane`.
#[must_use]
pub fn key(extension: &str, view: &str, pane: &str) -> String {
    format!("{PREFIX}{extension}/{view}#{pane}")
}

/// The extension and view a session key names.
#[must_use]
pub fn of_key(key: &str) -> Option<(&str, &str)> {
    let rest = key.strip_prefix(PREFIX)?;
    let (ext_view, _) = rest.rsplit_once('#')?;
    ext_view.rsplit_once('/')
}

/// What [`ProcessViewPane`] runs for `launch`.
#[must_use]
pub fn program(launch: &Launch) -> Program {
    let program = Program::new(&launch.program)
        .args(launch.args.iter().cloned())
        .dir(&launch.dir)
        .stamp(&launch.stamp)
        .restart(Restart::default());
    // The lab and tests force shared memory the same way for every view.
    match std::env::var(TRANSPORT_ENV) {
        Ok(t) => program.env(TRANSPORT_ENV, t),
        Err(_) => program,
    }
}

/// Starts (or finds, or starts again with a new stamp) extension
/// `extension`'s process view `view` for pane `pane`: what a pane does
/// when it opens, without a window.
///
/// # Errors
///
/// Why it cannot run (`architect_extensions::process_view::launch`).
pub fn open(host: &Host, extension: &str, view: &str, pane: &str) -> Result<Arc<Session>, String> {
    let launch = launch(host, extension, view)?;
    Ok(host::session(
        &key(extension, view, pane),
        &program(&launch),
    ))
}

/// Stops each extension's session no pane has shown for [`PARKED`], each
/// whose extension is no longer loaded, and each it may no longer run (a
/// grant taken back). Returns the keys ended.
#[allow(clippy::must_use_candidate)]
pub fn reap(host: &Host) -> Vec<String> {
    host::reap(PARKED, |key| match of_key(key) {
        Some((extension, view)) => launch(host, extension, view).is_ok(),
        // Not an extension's (the lab's own): kept.
        None => true,
    })
}

/// Provides `ProcessViews` for the app's window over `host`'s
/// extensions, and reaps their sessions. Call it once, near the app's
/// root (a pop-out window shares the first's sessions).
#[allow(clippy::must_use_candidate)]
pub fn use_process_views(host: Host) -> ProcessViews {
    let reaper = host.clone();
    use_future(move || {
        let host = reaper.clone();
        async move {
            loop {
                futures_timer::Delay::new(REAP).await;
                let _ = reap(&host);
            }
        }
    });
    let provided = use_hook(move || {
        ProcessViews(Rc::new(move |ask: &ProcessAsk| {
            let host = HostRef(host.clone());
            let ask = ask.clone();
            rsx! { ExtensionProcessPane { ask, host } }
        }))
    });
    use_context_provider(|| provided.clone());
    provided
}

/// The extensions' host, as a prop (one per app: always equal).
#[derive(Clone)]
pub struct HostRef(pub Host);

impl PartialEq for HostRef {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// One extension's process view in its pane, or why it cannot run.
#[component]
fn ExtensionProcessPane(ask: ProcessAsk, host: HostRef) -> Element {
    let pane = use_pane();
    if pane.is_preview() {
        // A card in a phone's tab switcher: never a second program.
        return rsx! {
            div { style: "flex: 1; display: flex; align-items: center; justify-content: center; \
                          color: var(--muted-foreground); font-size: 13px;",
                "{ask.view}"
            }
        };
    }
    match launch(&host.0, &ask.extension, &ask.view) {
        Ok(launch) => {
            let session_key = key(&ask.extension, &ask.view, &pane.id.0);
            let program = program(&launch);
            rsx! {
                div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
                    ProcessViewPane { key: "{session_key}", session_key, program }
                }
            }
        }
        Err(why) => rsx! {
            div { style: "flex: 1; display: flex; align-items: center; justify-content: center; padding: 24px; \
                          font-size: 13px; color: var(--muted-foreground); text-align: center;",
                "{ask.extension}'s view {ask.view} cannot be shown here: {why}"
            }
        },
    }
}
