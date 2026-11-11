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
mod view;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use architect_extensions::contrib::{Contribution, StatusDecl};
use architect_extensions::hostcall::HostRequest;
use architect_extensions::linked::{Builder, Extension, Handle};
use serde_json::{Value, json};

use crate::model::{Channel, State};

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

/// The releases page, when a release names none.
pub const RELEASES_PAGE: &str = "https://github.com/FastTrackStudios/architect/releases";

#[derive(Debug, Default)]
struct Shared {
    state: State,
    /// RFC 3339, the last check that reached the releases API.
    last_check: Option<String>,
    channel: Channel,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    state: State::Idle,
    last_check: None,
    channel: Channel::Stable,
});

static REVISION: AtomicU64 = AtomicU64::new(0);

static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

fn shared() -> std::sync::MutexGuard<'static, Shared> {
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

/// Sets the state and redraws the status item and the settings row.
fn set_state(state: State) {
    let line = model::status_line(&state);
    shared().state = state;
    REVISION.fetch_add(1, Ordering::Relaxed);
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

/// `UPDATES_CHECK`: check now.
async fn check(_: Value) -> Result<Value, String> {
    let channel = channel().await;
    if matches!(
        state(),
        State::Checking | State::Downloading { .. } | State::Ready { .. }
    ) {
        return Ok(status());
    }
    run::check(channel).await;
    Ok(status())
}

/// `UPDATES_SCHEDULED`: a check, when automatic checks are on.
async fn scheduled(_: Value) -> Result<Value, String> {
    if setting(settings::AUTO).await.as_bool() == Some(false) {
        return Ok(json!({ "checked": false, "why": "automatic checks are off" }));
    }
    check(Value::Null).await
}

/// `UPDATES_INSTALL`: start the download of the available update.
async fn install(_: Value) -> Result<Value, String> {
    // An unsigned release goes on: the engine refuses it, and says why.
    let (version, plan, notes_url) = match state() {
        State::Available {
            version,
            plan,
            notes_url,
            ..
        } => (version, plan, notes_url),
        State::Downloading { .. } | State::Ready { .. } => return Ok(status()),
        State::DevelopmentBuild => {
            return Err(model::ReportOnly::DevelopmentBuild.reason());
        }
        _ => return Err("no update is available: check first (UPDATES_CHECK)".to_owned()),
    };
    if let model::Plan::ReportOnly(why) = &plan {
        return Err(format!("{}: {}", why.reason(), page_of(&notes_url)));
    }
    run::install(version, plan)?;
    Ok(status())
}

fn page_of(notes_url: &str) -> String {
    if notes_url.is_empty() {
        RELEASES_PAGE.to_owned()
    } else {
        notes_url.to_owned()
    }
}

/// `UPDATES_PAGE`: the release's page (else every release's).
async fn page(_: Value) -> Result<Value, String> {
    let url = match state() {
        State::Available { notes_url, .. } => page_of(&notes_url),
        _ => RELEASES_PAGE.to_owned(),
    };
    host(HostRequest::Open { path: url.clone() }).await?;
    Ok(json!({ "opened": url }))
}

/// `UPDATES_RESTART`: into the installed update.
async fn restart(_: Value) -> Result<Value, String> {
    let State::Ready { version, plan } = state() else {
        return Err("no update is installed yet: nothing to restart into".to_owned());
    };
    let restarts = match &plan {
        model::Plan::InPlace { appimage } => {
            run::relaunch_after_exit(appimage)?;
            true
        }
        _ => false,
    };
    host(HostRequest::Run {
        action: "PRESENCE_QUIT".to_owned(),
        args: Value::Null,
        ticket: None,
    })
    .await?;
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

    use super::{REVISION, State, set_state, shared};
    use crate::engine::{ARCHITECT, Engine};
    use crate::model::{Channel, Env, Plan, Platform, percent};

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
        Env::from_vars(|k| std::env::var_os(k), crate::engine::writable)
    }

    const fn plan_name(plan: &Plan) -> &'static str {
        match plan {
            Plan::UpdateDir { .. } => "update_dir",
            Plan::InPlace { .. } => "in_place",
            Plan::ReportOnly(_) => "report_only",
        }
    }

    pub async fn check(channel: Channel) {
        let engine = engine();
        if engine.current().is_none() {
            set_state(State::DevelopmentBuild);
            return;
        }
        set_state(State::Checking);
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
        if !matches!(state, State::Failed { .. }) {
            shared().last_check = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true));
        }
        set_state(state);
    }

    pub fn install(version: String, plan: Plan) -> Result<(), String> {
        let rt = runtime()?;
        set_state(State::Downloading {
            version: version.clone(),
            received: 0,
            total: None,
        });
        let engine = engine();
        rt.spawn(async move {
            let started = Instant::now();
            let mut shown = None;
            let v = version.clone();
            let mut progress = move |received: u64, total: Option<u64>| {
                let now = percent(received, total);
                if now != shown {
                    shown = now;
                    set_state(State::Downloading {
                        version: v.clone(),
                        received,
                        total,
                    });
                } else if let State::Downloading { received: r, .. } = &mut shared().state {
                    *r = received;
                    REVISION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            };
            let done = engine.install(&version, &plan, &mut progress).await;
            let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            match done {
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
                    set_state(state);
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
                    set_state(State::Failed { reason: f.reason });
                }
            }
        });
        Ok(())
    }

    /// Start `appimage` once this process has exited.
    #[cfg(unix)]
    pub fn relaunch_after_exit(appimage: &std::path::Path) -> Result<(), String> {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec "$0""#)
            .arg(appimage)
            .arg(std::process::id().to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("start {} again: {e}", appimage.display()))
    }

    #[cfg(not(unix))]
    pub fn relaunch_after_exit(_: &std::path::Path) -> Result<(), String> {
        Err("restarting into an update is not supported on this platform yet".to_owned())
    }
}

/// Without feature `release`: nothing reaches the network.
#[cfg(not(feature = "release"))]
mod run {
    use super::{State, set_state};
    use crate::model::{Channel, Plan};

    const NO: &str = "this build of Architect cannot update itself (built without `release`)";

    // Async as the network half's is, so callers are the same.
    #[allow(clippy::unused_async)]
    pub async fn check(_: Channel) {
        if super::release_version().is_none() {
            set_state(State::DevelopmentBuild);
        } else {
            set_state(State::Failed {
                reason: NO.to_owned(),
            });
        }
    }

    pub fn install(_: String, _: Plan) -> Result<(), String> {
        Err(NO.to_owned())
    }

    pub fn relaunch_after_exit(_: &std::path::Path) -> Result<(), String> {
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
