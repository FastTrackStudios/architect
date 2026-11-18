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
//! - starts it again when what it runs changes ([`refresh`]): its stamp
//!   (a development copy rebuilt, which `EXT_DEV` reloads; another copy
//!   loaded), or an argument that names a setting
//!   (`{{setting:<key>}}`) when the setting changes; and when it dies, as
//!   a process extension's program is ([`Restart::default`]), saying why;
//! - stops it when its pane is gone from the workspace (closed: a move to
//!   another group, a split or a pop-out window keeps the pane, so the
//!   program runs on), at once when its extension unloads or its grant is
//!   taken back ([`reap`]). A pane drawn where there is no workspace to ask
//!   stops [`BACKSTOP`] after it went.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use architect_extensions::host::Host;
use architect_extensions::interface::{ProcessAsk, ProcessViews};
use architect_extensions::process_view::{Launch, launch};
use architect_workspace::use_pane;
use dioxus::prelude::*;

use crate::host::{self, ProcessViewPane, Program, Restart, Session};

/// What forces a program's frames over shared memory (`shm`), as for the
/// lab's (`guest::TRANSPORT_ENV`).
pub const TRANSPORT_ENV: &str = "ARCHITECT_PROCESS_VIEW_TRANSPORT";

/// The prefix of extensions' sessions' keys.
pub const PREFIX: &str = "ext:";

/// The backstop: how long a session whose pane went away is kept.
///
/// For when nothing says whether the pane is still in a workspace (a view
/// drawn outside one). A pane the workspace knows is kept while it is there.
pub const BACKSTOP: Duration = Duration::from_secs(2);

/// How often sessions are checked against the host and the workspace.
pub const REAP: Duration = Duration::from_millis(250);

/// Whether a session's pane is still in its workspace, by session key:
/// registered by the pane as it mounts (the window's thread; a pop-out's
/// panes are on the same one).
type Present = Rc<dyn Fn() -> bool>;

thread_local! {
    static PANES: RefCell<HashMap<String, Present>> = RefCell::new(HashMap::new());
}

/// Says how to tell whether `key`'s pane is still in its workspace (a pane
/// does it as it mounts; a test with no workspace gives its own).
pub fn register_pane(key: &str, present: impl Fn() -> bool + 'static) {
    PANES.with(|p| p.borrow_mut().insert(key.to_owned(), Rc::new(present)));
}

/// What becomes of a session, from what is known of its pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fate {
    Keep,
    End,
}

/// A session's fate.
///
/// Shown (`away` none) it is kept; away from its pane, it
/// ends when the workspace says the pane is gone (`present`
/// `Some(false)`), is kept while the pane is there (a move in flight, a
/// tab behind another: it draws nothing until shown, as it is ticked only
/// when a frame is shown), and with no workspace to ask ends after
/// [`BACKSTOP`].
#[must_use]
pub fn fate(away: Option<Duration>, present: Option<bool>) -> Fate {
    match (away, present) {
        (Some(_), Some(false)) => Fate::End,
        (Some(away), None) if away > BACKSTOP => Fate::End,
        _ => Fate::Keep,
    }
}

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

/// The extensions' sessions' keys.
fn keys() -> Vec<String> {
    host::keys()
        .into_iter()
        .filter(|k| k.starts_with(PREFIX))
        .collect()
}

/// Stops extensions' sessions that should not run.
///
/// Each whose pane is gone ([`fate`]), whose
/// extension is no longer loaded, or that may no longer run its program (a
/// grant taken back). The lab's own sessions are not touched. Returns the
/// keys ended.
#[allow(clippy::must_use_candidate)]
pub fn reap(host: &Host) -> Vec<String> {
    let mut ended = Vec::new();
    for key in keys() {
        let present = PANES.with(|p| p.borrow().get(&key).cloned()).map(|f| f());
        let why = match of_key(&key).map(|(e, v)| launch(host, e, v)) {
            Some(Ok(_)) => {
                (fate(host::away(&key), present) == Fate::End).then(|| "its pane closed".to_owned())
            }
            Some(Err(why)) => Some(why),
            None => Some("not an extension's view".to_owned()),
        };
        if let Some(why) = why {
            host::retire(&key, &why);
            PANES.with(|p| p.borrow_mut().remove(&key));
            ended.push(key);
        }
    }
    ended
}

/// Starts again sessions whose program changed.
///
/// Each extension's session whose program, arguments or stamp
/// changed since it started (a setting its arguments name; a development
/// copy rebuilt, even with no pane redrawn). Returns the keys started again.
#[allow(clippy::must_use_candidate)]
pub fn refresh(host: &Host) -> Vec<String> {
    let mut again = Vec::new();
    for key in keys() {
        let Some((extension, view)) = of_key(&key) else {
            continue;
        };
        let (Ok(now), Some(session)) = (launch(host, extension, view), host::find(&key)) else {
            continue;
        };
        let program = program(&now);
        if session.program() != program {
            let _ = host::session(&key, &program);
            again.push(key);
        }
    }
    again
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
                let _ = refresh(&host);
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

/// How often an open pane checks again whether its view may run (a grant
/// given or taken back, a program built).
const CHECK: Duration = Duration::from_millis(500);

/// One extension's process view in its pane, or why it cannot run: with
/// an Allow button when all it lacks is a person's yes to run its program.
#[component]
fn ExtensionProcessPane(ask: ProcessAsk, host: HostRef) -> Element {
    let pane = use_pane();
    let checked = {
        let (host, ask) = (host.0.clone(), ask.clone());
        move || launch(&host, &ask.extension, &ask.view).map(|l| program(&l))
    };
    let mut failed = use_signal(|| None::<String>);
    let mut now = use_signal({
        let checked = checked.clone();
        move || checked()
    });
    use_future({
        let checked = checked.clone();
        move || {
            let checked = checked.clone();
            async move {
                loop {
                    futures_timer::Delay::new(CHECK).await;
                    let next = checked();
                    if *now.peek() != next {
                        now.set(next);
                    }
                }
            }
        }
    });
    if pane.is_preview() {
        // A card in a phone's tab switcher: never a second program.
        return rsx! {
            div { style: "flex: 1; display: flex; align-items: center; justify-content: center; \
                          color: var(--muted-foreground); font-size: 13px;",
                "{ask.view}"
            }
        };
    }
    let session_key = key(&ask.extension, &ask.view, &pane.id.0);
    // The reaper asks the workspace whether this pane is still there: a
    // move keeps it (the program runs on), a close does not.
    let (workspace, id) = (pane.workspace(), pane.id);
    register_pane(&session_key, move || workspace.peek().pane(&id).is_some());
    match now() {
        Ok(program) => rsx! {
            div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
                ProcessViewPane { key: "{session_key}", session_key, program }
            }
        },
        Err(why) => {
            let allow =
                architect_extensions::process_view::needs_grant(&host.0, &ask.extension, &ask.view);
            let says = allow.as_ref().map(|p| {
                format!("Run {p}, a program in its package, on this computer to draw this view?")
            });
            let grant = {
                let (host, ask) = (host.0, ask.clone());
                move |_| {
                    let Some(program) = allow.clone() else { return };
                    match architect_extensions::process_view::grant_program(
                        &host,
                        &ask.extension,
                        &program,
                        "person",
                    ) {
                        Ok(()) => now.set(checked()),
                        Err(e) => failed.set(Some(e)),
                    }
                }
            };
            rsx! {
                div { style: "flex: 1; display: flex; flex-direction: column; align-items: center; justify-content: center; \
                              gap: 12px; padding: 24px; font-size: 13px; color: var(--muted-foreground); text-align: center;",
                    if let Some(says) = says {
                        div { style: "font-size: 14px; font-weight: 600; color: var(--foreground);",
                            "{ask.extension} asks to run a program"
                        }
                        div { style: "max-width: 420px;", "{says}" }
                        button {
                            style: "padding: 6px 14px; border-radius: 6px; border: 1px solid var(--border); \
                                    background: var(--primary); color: var(--primary-foreground); font-size: 13px;",
                            onclick: grant,
                            "Allow"
                        }
                        if let Some(e) = failed() {
                            div { style: "color: var(--destructive);", "{e}" }
                        }
                    } else {
                        div { "{ask.extension}'s view {ask.view} cannot be shown here: {why}" }
                    }
                }
            }
        }
    }
}
