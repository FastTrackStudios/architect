//! Updates (`features/updates/extension.toml`, #523): the Architect app
//! keeps itself on the newest signed release.
//!
//! - **Check** at start and every 24 h while automatic checks are on
//!   (`updates.auto`), or on `UPDATES_CHECK`: the newest `vX.Y.Z` release
//!   on the channel (`updates.channel`; Stable, Nightly refused until it
//!   exists) against the running release version ([`RELEASE_VERSION`]).
//! - **Install** only on `UPDATES_INSTALL`: download, verify against the
//!   org-signed `SHA256SUMS` (`architect::release`), and put it where
//!   [`model::plan`] says: the Nix launcher's `$ARCHITECT_UPDATE_DIR`, or
//!   over `$APPIMAGE`, else report the update and its page. An unsigned
//!   release is reported, never installed.
//! - **Show** one quiet status item ("Update X available", then "Restart
//!   to finish") and a settings section with the version, channel, last
//!   check and Check now.
//!
//! A build not stamped with a release version (`cargo run`, CI) is a
//! development build: it never checks, offers or installs anything.
//!
//! The network half is feature `release` (the app turns it on, as it does
//! `architect-extensions/net`); without it the actions answer that this
//! build cannot update itself.

#[cfg(feature = "release")]
pub mod engine;
pub mod model;
pub mod restart;
mod view;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use architect_extensions::contrib::{Contribution, StatusDecl};
use architect_extensions::hostcall::HostRequest;
use architect_extensions::linked::{Builder, Extension, Handle};
use serde_json::{Value, json};

use crate::model::{Begin, Channel, Machine, State};

/// Its id: its actions are `UPDATES_*`.
pub const ID: &str = "updates";

/// The release version stamped in at build time.
///
/// `cargo xtask dist desktop --version <X.Y.Z>` sets the build-time env
/// `ARCHITECT_RELEASE_VERSION`; read here and nowhere else, so only this
/// crate's cache key moves with it. `None`: a development build.
pub const RELEASE_VERSION: Option<&str> = option_env!("ARCHITECT_RELEASE_VERSION");

/// The running release version (`X.Y.Z`), or `None` for a development
/// build. Never `CARGO_PKG_VERSION`.
#[must_use]
pub fn release_version() -> Option<String> {
    model::release_version(RELEASE_VERSION)
}

/// Its action ids.
pub mod actions {
    /// Check now: the state after.
    pub const CHECK: &str = "UPDATES_CHECK";
    /// Download, verify and install the available update.
    pub const INSTALL: &str = "UPDATES_INSTALL";
    /// Where the updater is.
    pub const STATE: &str = "UPDATES_STATE";
    /// Restart (or quit) into the installed update.
    pub const RESTART: &str = "UPDATES_RESTART";
    /// Open the release's page.
    pub const PAGE: &str = "UPDATES_PAGE";
    /// Its hooks' target: check when automatic checks are on.
    pub const SCHEDULED: &str = "UPDATES_SCHEDULED";
}

/// Its settings.
pub mod settings {
    /// `stable` or `nightly`.
    pub const CHANNEL: &str = "updates.channel";
    /// Check at start and daily.
    pub const AUTO: &str = "updates.auto";
}

/// Its status item's id.
pub const STATUS: &str = "update";

/// The settings section's view.
pub const SETTINGS_VIEW: &str = "settings";

pub use crate::model::RELEASES_PAGE;

static SHARED: Mutex<Machine> = Mutex::new(Machine::new());

static REVISION: AtomicU64 = AtomicU64::new(0);

static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

fn shared() -> std::sync::MutexGuard<'static, Machine> {
    SHARED.lock().unwrap_or_else(PoisonError::into_inner)
}

fn handle() -> Option<Handle> {
    HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn state() -> State {
    shared().state.clone()
}

/// Apply one transition under the lock; when it changed the state, redraw
/// the status item and the settings row.
fn transition<T>(change: impl FnOnce(&mut Machine) -> T, changed: impl Fn(&T) -> bool) -> T {
    let mut m = shared();
    let out = change(&mut m);
    let now = changed(&out).then(|| m.state.clone());
    drop(m);
    if let Some(state) = now {
        publish(&state);
    }
    out
}

/// Redraw what shows `state`: the status item and the settings row.
fn publish(state: &State) {
    REVISION.fetch_add(1, Ordering::Relaxed);
    let line = model::status_line(state);
    if let Some(h) = handle() {
        h.contribute(Contribution {
            status: line
                .into_iter()
                .map(|l| StatusDecl {
                    id: STATUS.to_owned(),
                    text: l.text,
                    icon: "download".to_owned(),
                    hint: l.hint,
                    action: l.action.to_owned(),
                    ..StatusDecl::default()
                })
                .collect(),
            ..Contribution::default()
        });
    }
}

/// What `UPDATES_STATE` answers.
fn status() -> Value {
    let s = shared();
    json!({
        "current": release_version(),
        "development_build": release_version().is_none(),
        "channel": s.channel.as_str(),
        "last_check": s.last_check,
        "update": s.state,
    })
}

async fn host(request: HostRequest) -> Result<Value, String> {
    let h = handle().ok_or("updates is not running")?;
    h.host(request).await.map_err(|e| e.to_string())
}

async fn setting(key: &str) -> Value {
    host(HostRequest::Setting {
        key: key.to_owned(),
    })
    .await
    .unwrap_or(Value::Null)
}

async fn channel() -> Channel {
    let c = setting(settings::CHANNEL)
        .await
        .as_str()
        .map_or(Channel::Stable, Channel::parse);
    shared().channel = c;
    c
}

/// `UPDATES_CHECK`: check now (not while an install is under way or done).
async fn check(_: Value) -> Result<Value, String> {
    let channel = channel().await;
    if transition(Machine::begin_check, |began| *began) {
        let (state, at) = run::check(channel).await;
        transition(|m| m.finish_check(state, at), |kept| *kept);
    }
    Ok(status())
}

/// `UPDATES_SCHEDULED`: a check, when automatic checks are on.
async fn scheduled(_: Value) -> Result<Value, String> {
    if setting(settings::AUTO).await.as_bool() == Some(false) {
        return Ok(json!({ "checked": false, "why": "automatic checks are off" }));
    }
    check(Value::Null).await
}

/// `UPDATES_INSTALL`: start the download of the available update, once.
async fn install(_: Value) -> Result<Value, String> {
    // An unsigned release goes on: the engine refuses it, and says why.
    match transition(Machine::begin_install, |b| {
        matches!(b, Ok(Begin::Start { .. }))
    })? {
        Begin::Start { version, plan } => {
            if let Err(why) = run::install(version, plan) {
                transition(
                    |m| {
                        m.finish_install(State::Failed {
                            reason: why.clone(),
                        })
                    },
                    |kept| *kept,
                );
                return Err(why);
            }
        }
        Begin::Already => {}
    }
    Ok(status())
}

/// `UPDATES_PAGE`: the release's page (else every release's), in the
/// system's browser; only a page under [`RELEASES_PAGE`].
async fn page(_: Value) -> Result<Value, String> {
    let url = match state() {
        State::Available { notes_url, .. } => model::release_page(&notes_url).to_owned(),
        _ => RELEASES_PAGE.to_owned(),
    };
    restart::open_in_browser(&url)?;
    Ok(json!({ "opened": url }))
}

/// `UPDATES_RESTART`: into the installed update.
async fn restart(_: Value) -> Result<Value, String> {
    let State::Ready { version, plan } = state() else {
        return Err("no update is installed yet: nothing to restart into".to_owned());
    };
    let watcher = match &plan {
        model::Plan::InPlace { appimage } => Some(restart::start_after_exit(
            appimage,
            std::process::id(),
            restart::MAX_WAIT,
        )?),
        _ => None,
    };
    let restarts = watcher.is_some();
    let quit = host(HostRequest::Run {
        action: "PRESENCE_QUIT".to_owned(),
        args: Value::Null,
        ticket: None,
    })
    .await;
    match (quit, watcher) {
        (Err(why), Some(w)) => {
            // The app is not quitting: never start it again later.
            w.cancel();
            return Err(why);
        }
        (Err(why), None) => return Err(why),
        (Ok(_), Some(w)) => w.detach(),
        (Ok(_), None) => {}
    }
    Ok(json!({
        "version": version,
        "restarting": restarts,
        "next": if restarts { "Architect starts again on the new version" } else { "open Architect again to run the new version" },
    }))
}

/// The network half: checks and installs on a runtime of its own.
#[cfg(feature = "release")]
mod run {
    use std::sync::OnceLock;
    use std::time::Instant;

    use chrono::{SecondsFormat, Utc};

    use super::{State, transition};
    use crate::engine::{ARCHITECT, Engine};
    use crate::model::{Channel, Env, Plan, Platform};

    fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
        static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
        RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("updates")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(Clone::clone)
    }

    fn engine() -> Engine {
        Engine::new(&ARCHITECT, super::release_version(), Platform::this())
    }

    /// The process's env, read here at the edge.
    fn env() -> Env {
        let exe = std::env::current_exe().and_then(std::fs::canonicalize).ok();
        Env::from_vars(
            |k| std::env::var_os(k),
            exe.as_deref(),
            crate::engine::writable,
        )
    }

    const fn plan_name(plan: &Plan) -> &'static str {
        match plan {
            Plan::UpdateDir { .. } => "update_dir",
            Plan::InPlace { .. } => "in_place",
            Plan::ReportOnly(_) => "report_only",
        }
    }

    /// The check's answer and, when it reached the API, when.
    pub async fn check(channel: Channel) -> (State, Option<String>) {
        let engine = engine();
        if engine.current().is_none() {
            return (State::DevelopmentBuild, None);
        }
        let started = Instant::now();
        let current = engine.current().map(str::to_owned);
        let work = async move { engine.check(channel, &env()).await };
        let state = match runtime() {
            Ok(rt) => rt.spawn(work).await.unwrap_or_else(|e| State::Failed {
                reason: e.to_string(),
            }),
            Err(reason) => State::Failed { reason },
        };
        let (outcome, newest, error) = match &state {
            State::Available { version, .. } => ("available", Some(version.as_str()), None),
            State::UpToDate { newest } => ("up_to_date", newest.as_deref(), None),
            State::Failed { reason } => ("failed", None, Some(reason.as_str())),
            _ => ("other", None, None),
        };
        // One event per check; a failure (offline) is quiet: info.
        tracing::info!(
            target: "architect_updates",
            run = "check",
            outcome,
            current = current.as_deref(),
            newest,
            channel = channel.as_str(),
            error,
            duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        let at = (!matches!(state, State::Failed { .. }))
            .then(|| Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
        (state, at)
    }

    /// Start the download on the runtime; the state moves as it goes.
    pub fn install(version: String, plan: Plan) -> Result<(), String> {
        let rt = runtime()?;
        let engine = engine();
        rt.spawn(async move {
            let started = Instant::now();
            let v = version.clone();
            let mut progress = move |received: u64, total: Option<u64>| {
                transition(|m| m.progress(&v, received, total), |moved| *moved);
            };
            let done = engine.install(&version, &plan, &mut progress).await;
            let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            let state = match done {
                Ok(state) => {
                    tracing::info!(
                        target: "architect_updates",
                        run = "install",
                        outcome = "installed",
                        current = engine.current(),
                        version,
                        plan = plan_name(&plan),
                        duration_ms,
                    );
                    state
                }
                Err(f) => {
                    // A refused release is alertable; a dropped download is not.
                    if f.refused {
                        tracing::error!(
                            target: "architect_updates",
                            run = "install",
                            outcome = "refused",
                            current = engine.current(),
                            version,
                            plan = plan_name(&plan),
                            error = f.reason.as_str(),
                            duration_ms,
                        );
                    } else {
                        tracing::info!(
                            target: "architect_updates",
                            run = "install",
                            outcome = "failed",
                            current = engine.current(),
                            version,
                            plan = plan_name(&plan),
                            error = f.reason.as_str(),
                            duration_ms,
                        );
                    }
                    State::Failed { reason: f.reason }
                }
            };
            transition(|m| m.finish_install(state), |kept| *kept);
        });
        Ok(())
    }
}

/// Without feature `release`: nothing reaches the network.
#[cfg(not(feature = "release"))]
mod run {
    use super::State;
    use crate::model::{Channel, Plan};

    const NO: &str = "this build of Architect cannot update itself (built without `release`)";

    // Async as the network half's is, so callers are the same.
    #[allow(clippy::unused_async)]
    pub async fn check(_: Channel) -> (State, Option<String>) {
        if super::release_version().is_none() {
            (State::DevelopmentBuild, None)
        } else {
            (
                State::Failed {
                    reason: NO.to_owned(),
                },
                None,
            )
        }
    }

    pub fn install(_: String, _: Plan) -> Result<(), String> {
        Err(NO.to_owned())
    }
}

/// The extension: the Architect app's updater.
#[derive(Debug, Clone, Copy, Default)]
pub struct Updates;

impl Extension for Updates {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        *HANDLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(ext.handle());
        if release_version().is_none() {
            shared().state = State::DevelopmentBuild;
        }
        ext.view(SETTINGS_VIEW, view::Settings)
            .action(actions::CHECK, check)
            .action(actions::SCHEDULED, scheduled)
            .action(actions::INSTALL, install)
            .action(actions::STATE, |_| async {
                let _ = channel().await;
                Ok(status())
            })
            .action(actions::RESTART, restart)
            .action(actions::PAGE, page);
    }
}
