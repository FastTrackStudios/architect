//! The sandbox boot: a tempdir as the whole world a test's app can see.
//!
//! Why this exists: an app resolves its data, config and cache dirs from
//! the environment (`HOME`, the `XDG_*` dirs, `APPDATA`, its own
//! `<APP>_DATA_ROOT`). A test that boots the app as-is on a developer's
//! machine boots it over the developer's real data: Task's e2e binaries
//! once booted every real org, raced on its storage registry and wrote
//! markers into real vaults. A test that reads the developer's disk is a
//! different program on every machine.
//!
//! [`sandbox_server`] closes that door for the boot: under [`env_lock`] it
//! points every variable in [`HOME_VARS`] inside a fresh tempdir, runs the
//! caller's `seed` (which plants fixtures and builds the app), then serves
//! the app on `127.0.0.1:0`.

use std::ffi::{OsStr, OsString};
use std::future::Future;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use axum::Router;

use crate::LayerRouter;
use crate::host::EngineHost;

/// THE lock around the process environment, for every test in a binary.
///
/// `cargo test` runs tests on a shared thread pool and the environment is
/// one per process: a lock per test file serializes nothing across files.
/// Every sandbox boot takes this one.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The per-user directory variables a sandbox points inside itself, and
/// where (relative to [`Sandbox::home`]):
///
/// | variable | sandbox path |
/// |---|---|
/// | `HOME`, `USERPROFILE` | `home/` |
/// | `XDG_CONFIG_HOME` | `home/.config` |
/// | `XDG_DATA_HOME` | `home/.local/share` |
/// | `XDG_CACHE_HOME` | `home/.cache` |
/// | `XDG_STATE_HOME` | `home/.local/state` |
/// | `XDG_RUNTIME_DIR` | `home/.run` |
/// | `APPDATA` | `home/AppData/Roaming` |
/// | `LOCALAPPDATA` | `home/AppData/Local` |
///
/// On macOS the platform dirs (`~/Library/Application Support`,
/// `~/Library/Caches`) derive from `HOME`, so they land inside too.
pub const HOME_VARS: &[(&str, &str)] = &[
    ("HOME", ""),
    ("USERPROFILE", ""),
    ("XDG_CONFIG_HOME", ".config"),
    ("XDG_DATA_HOME", ".local/share"),
    ("XDG_CACHE_HOME", ".cache"),
    ("XDG_STATE_HOME", ".local/state"),
    ("XDG_RUNTIME_DIR", ".run"),
    ("APPDATA", "AppData/Roaming"),
    ("LOCALAPPDATA", "AppData/Local"),
];

/// The held [`env_lock`]: the only safe place to change the environment.
///
/// Setting a variable while another thread reads the environment is
/// undefined behaviour on most platforms, so every write a test makes
/// goes through this guard, and every boot that reads the environment
/// runs while one is held.
#[derive(Debug)]
pub struct EnvGuard {
    _held: tokio::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    /// Set `name` to `value` for the process.
    pub fn set(&mut self, name: &str, value: impl AsRef<OsStr>) {
        // SAFETY: the process-wide env lock is held for as long as `self`
        // lives, and every sandbox boot reads the environment under it.
        unsafe { std::env::set_var(name, value) };
    }

    /// Remove `name` from the process environment.
    pub fn remove(&mut self, name: &str) {
        // SAFETY: as in `set`.
        unsafe { std::env::remove_var(name) };
    }
}

/// Take the process environment lock. Hold it while setting variables
/// and constructing whatever reads them.
pub async fn env_lock() -> EnvGuard {
    EnvGuard {
        _held: ENV_LOCK.lock().await,
    }
}

/// Where a sandboxed boot lives. Cheap to clone; the directory itself is
/// owned by the [`Sandboxed`] or [`SandboxServer`] the boot returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sandbox {
    root: PathBuf,
}

impl Sandbox {
    /// The tempdir everything lives under.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The fake home: `HOME` (and `USERPROFILE`) point here.
    #[must_use]
    pub fn home(&self) -> PathBuf {
        self.root.join("home")
    }

    /// A data root for the app, beside the home. Empty until `seed`
    /// writes to it; [`SandboxBuilder::data_env`] names it to the app.
    #[must_use]
    pub fn data(&self) -> PathBuf {
        self.root.join("data")
    }

    /// Whether `path` is inside this sandbox (lexically, no symlinks).
    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        path.starts_with(&self.root)
    }

    /// The value a sandbox gives each of [`HOME_VARS`].
    #[must_use]
    pub fn home_env(&self) -> Vec<(&'static str, PathBuf)> {
        let home = self.home();
        HOME_VARS
            .iter()
            .map(|(name, rel)| {
                (
                    *name,
                    if rel.is_empty() {
                        home.clone()
                    } else {
                        home.join(rel)
                    },
                )
            })
            .collect()
    }

    fn lay_out(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(self.data())?;
        for (_, dir) in self.home_env() {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

/// A new sandbox in a fresh tempdir, and the owner that deletes it.
fn fresh_sandbox() -> std::io::Result<(Sandbox, Option<tempfile::TempDir>)> {
    let dir = tempfile::Builder::new()
        .prefix("architect-sandbox-")
        .tempdir()?;
    let sandbox = Sandbox {
        root: dir.path().to_owned(),
    };
    Ok((sandbox, Some(dir)))
}

/// A sandboxed boot's result: the value `seed` built, and the sandbox it
/// was built over. Dropping it deletes the directory (unless it boots
/// over a sandbox another value owns, see [`SandboxBuilder::over`]).
#[derive(Debug)]
pub struct Sandboxed<T> {
    /// What `seed` returned.
    pub value: T,
    /// Where it lives.
    pub sandbox: Sandbox,
    dir: Option<tempfile::TempDir>,
}

impl<T> Sandboxed<T> {
    /// Split off the value, keeping the directory alive in the returned
    /// guard. Drop the guard to delete the sandbox.
    #[must_use]
    pub fn into_parts(self) -> (T, Sandbox, Option<tempfile::TempDir>) {
        (self.value, self.sandbox, self.dir)
    }
}

/// What a sandbox can serve: an axum app, a vox router (mounted on an
/// [`EngineHost`] at `/vox`, with `/health` and `/version.json`), or a
/// configured [`EngineHost`].
pub trait IntoSandboxApp {
    /// The axum app to bind.
    fn into_sandbox_app(self) -> Router;
}

impl IntoSandboxApp for Router {
    fn into_sandbox_app(self) -> Router {
        self
    }
}

impl IntoSandboxApp for EngineHost {
    fn into_sandbox_app(self) -> Router {
        self.into_app()
    }
}

impl IntoSandboxApp for LayerRouter {
    fn into_sandbox_app(self) -> Router {
        EngineHost::new(self, "127.0.0.1:0").into_app()
    }
}

/// A sandboxed app served on an ephemeral loopback port. Dropping it
/// stops the server and deletes the sandbox.
#[derive(Debug)]
pub struct SandboxServer {
    addr: SocketAddr,
    sandbox: Sandbox,
    task: tokio::task::JoinHandle<()>,
    _dir: Option<tempfile::TempDir>,
}

impl SandboxServer {
    /// The bound address (`127.0.0.1:<port>`).
    #[must_use]
    pub const fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// The vox WebSocket URL: `ws://127.0.0.1:<port>/vox`.
    #[must_use]
    pub fn url(&self) -> String {
        format!("ws://{}/vox", self.addr)
    }

    /// The HTTP base URL: `http://127.0.0.1:<port>`.
    #[must_use]
    pub fn http_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Where it lives.
    #[must_use]
    pub const fn sandbox(&self) -> &Sandbox {
        &self.sandbox
    }
}

impl Drop for SandboxServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// How a sandbox boots: which of the app's own variables to clear or set
/// on top of [`HOME_VARS`]. Start with [`sandbox`].
#[derive(Debug, Default)]
pub struct SandboxBuilder {
    clear: Vec<String>,
    env: Vec<(String, OsString)>,
    data_env: Vec<String>,
    over: Option<Sandbox>,
}

/// A sandbox builder with nothing but [`HOME_VARS`] redirected.
#[must_use]
pub fn sandbox() -> SandboxBuilder {
    SandboxBuilder::default()
}

/// Boot an app over a fresh sandbox and serve it on `127.0.0.1:0`.
///
/// Under [`env_lock`], every variable in [`HOME_VARS`] is pointed inside a
/// new tempdir, then `seed` runs with the [`Sandbox`]: it plants fixtures
/// and returns what to serve ([`IntoSandboxApp`]). The lock is released
/// once `seed` returns, so an app that reads the environment at boot reads
/// the sandbox's. The variables stay set after the boot (a value read per
/// request still sees the sandbox), which is sound one test per process,
/// the way nextest runs.
///
/// ```ignore
/// let server = architect::testing::sandbox_server(|sb| async move {
///     plant_example_org(&sb.data())?;
///     let backend = Backend::open(&sb.data()).await?;
///     Ok(backend.into_router())
/// })
/// .await?;
/// let client = connect(&server.url()).await?;
/// ```
#[allow(clippy::future_not_send)] // as `Send` as the caller's seed; tests need not spawn it
pub async fn sandbox_server<F, Fut, A>(seed: F) -> eyre::Result<SandboxServer>
where
    F: FnOnce(Sandbox) -> Fut,
    Fut: Future<Output = eyre::Result<A>>,
    A: IntoSandboxApp,
{
    sandbox().server(seed).await
}

impl SandboxBuilder {
    /// Remove `name` before the boot: an app variable a developer's shell
    /// (or another test) may have set, like `TASK_SERVER_VAULT_ROOT`.
    #[must_use]
    pub fn clear(mut self, name: impl Into<String>) -> Self {
        self.clear.push(name.into());
        self
    }

    /// Set `name` to `value` for the boot.
    #[must_use]
    pub fn env(mut self, name: impl Into<String>, value: impl Into<OsString>) -> Self {
        self.env.push((name.into(), value.into()));
        self
    }

    /// Set `name` to the sandbox's [`Sandbox::data`] dir for the boot,
    /// like `TASK_DATA_ROOT`.
    #[must_use]
    pub fn data_env(mut self, name: impl Into<String>) -> Self {
        self.data_env.push(name.into());
        self
    }

    /// Boot over an existing sandbox instead of a fresh one: the restart
    /// shape, for asserting that whatever runs at startup is idempotent
    /// against its own output. The returned value does not own (or
    /// delete) the directory; keep the first boot's owner alive.
    #[must_use]
    pub fn over(mut self, sandbox: &Sandbox) -> Self {
        self.over = Some(sandbox.clone());
        self
    }

    /// Boot `seed` in the sandbox without serving it: for a test that
    /// drives the app's state in process.
    #[allow(clippy::future_not_send)] // as `Send` as the caller's seed
    #[allow(clippy::significant_drop_tightening)] // the lock must cover `seed`: it reads the env
    pub async fn boot<F, Fut, T>(self, seed: F) -> eyre::Result<Sandboxed<T>>
    where
        F: FnOnce(Sandbox) -> Fut,
        Fut: Future<Output = eyre::Result<T>>,
    {
        let (sandbox, dir) = self
            .over
            .map_or_else(fresh_sandbox, |sandbox| Ok((sandbox, None)))?;
        sandbox.lay_out()?;

        let mut env = env_lock().await;
        for name in &self.clear {
            env.remove(name);
        }
        for (name, value) in sandbox.home_env() {
            env.set(name, value);
        }
        for name in &self.data_env {
            env.set(name, sandbox.data());
        }
        for (name, value) in &self.env {
            env.set(name, value);
        }
        let value = seed(sandbox.clone()).await?;
        drop(env);

        Ok(Sandboxed {
            value,
            sandbox,
            dir,
        })
    }

    /// [`boot`](Self::boot), then serve what `seed` returned on
    /// `127.0.0.1:0`. See [`sandbox_server`].
    #[allow(clippy::future_not_send)] // as `Send` as the caller's seed
    pub async fn server<F, Fut, A>(self, seed: F) -> eyre::Result<SandboxServer>
    where
        F: FnOnce(Sandbox) -> Fut,
        Fut: Future<Output = eyre::Result<A>>,
        A: IntoSandboxApp,
    {
        let booted = self.boot(seed).await?;
        let (app, sandbox, dir) = booted.into_parts();
        let app = app.into_sandbox_app();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let task = tokio::spawn(async move {
            if let Err(e) = axum::serve(listener, app).await {
                tracing::warn!("sandbox server stopped: {e}");
            }
        });
        Ok(SandboxServer {
            addr,
            sandbox,
            task,
            _dir: dir,
        })
    }
}
