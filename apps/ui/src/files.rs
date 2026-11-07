//! The device's files agent in the app (#347): `FILES_*` on the list,
//! answered over the agent's HTTP face (`architect-desktop serve` with a
//! `files_dir`, bound to loopback), so the Files extension, the palette
//! and an agent over MCP reach the same roots, pins and versions.
//!
//! The HTTP client needs tokio and the window's executor is not, so each
//! call runs on a small runtime of its own ([`Agent`]).

use std::future::Future;
use std::sync::OnceLock;

use architect::action::ActionList;
use architect_files::agent::AgentError;
use architect_files::agent::files::{Files, FilesHttpClient, files_actions};
use architect_files::rendition::Peaks;
use architect_files::roots::{
    Asset, Checkpoint, DriveStatus, FileEntry, FileVersion, Hit, RootStatus,
};

/// Where the agent listens: `http://127.0.0.1:8090`.
pub const AGENT: &str = "ARCHITECT_FILES_AGENT";

/// `FILES_*` on `list`, when the device has an agent: the one
/// `ARCHITECT_FILES_AGENT` names, else the installed one (started, or
/// joined if it runs).
pub fn register(list: &ActionList) {
    let Some(base) = std::env::var(AGENT).ok().or_else(join_agent) else {
        return;
    };
    let agent = Agent(FilesHttpClient::at(base.trim_end_matches('/')));
    if let Err(why) = files_actions::register(agent, list) {
        tracing::warn!(?why, "FILES_* are not on the list");
    }
}

/// The installed agent's address, started if it is not running: through
/// its service when there is one, else as a detached `serve` (it outlives
/// the window: it is the device's sync agent).
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn join_agent() -> Option<String> {
    use architect_files::drive::agent as svc;
    let defaults = svc::Defaults::here()?;
    let text = std::fs::read_to_string(&defaults.config).ok()?;
    let bind = svc::get_key(&text, "bind")
        .unwrap_or_else(|| svc::BIND.to_owned())
        .replace("0.0.0.0", "127.0.0.1");
    let up = |addr: &str| {
        addr.parse::<std::net::SocketAddr>().is_ok_and(|a| {
            std::net::TcpStream::connect_timeout(&a, std::time::Duration::from_millis(300)).is_ok()
        })
    };
    if !up(&bind) {
        let started = svc::service_dir()
            .filter(|d| svc::installed(d))
            .map(|_| svc::start_installed());
        if !matches!(started, Some(Ok(()))) {
            spawn_agent(&defaults.config);
        }
    }
    Some(format!("http://{bind}"))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn join_agent() -> Option<String> {
    None
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn spawn_agent(config: &std::path::Path) {
    use std::os::unix::process::CommandExt as _;
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let spawned = std::process::Command::new(exe)
        .args(["serve", "--config"])
        .arg(config)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .process_group(0)
        .spawn();
    match spawned {
        Ok(child) => tracing::info!(pid = child.id(), "started the files agent"),
        Err(why) => tracing::warn!(%why, "the files agent did not start"),
    }
}

fn runtime() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("files-agent")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

async fn on_runtime<T: Send + 'static>(
    work: impl Future<Output = Result<T, AgentError>> + Send + 'static,
) -> Result<T, AgentError> {
    runtime()
        .map_err(AgentError::Transport)?
        .spawn(work)
        .await
        .map_err(|e| AgentError::Transport(e.to_string()))?
}

/// The agent's HTTP client, each call on [`runtime`].
#[derive(Clone)]
struct Agent(FilesHttpClient);

impl Files for Agent {
    async fn roots(&self) -> Result<Vec<RootStatus>, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::roots(&c).await }).await
    }
    async fn browse(&self, root: String, path: String) -> Result<Vec<FileEntry>, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::browse(&c, root, path).await }).await
    }
    async fn pin(&self, root: String, path: Option<String>) -> Result<String, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::pin(&c, root, path).await }).await
    }
    async fn unpin(&self, root: String) -> Result<(), AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::unpin(&c, root).await }).await
    }
    async fn history(&self, root: String, path: String) -> Result<Vec<FileVersion>, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::history(&c, root, path).await }).await
    }
    async fn restore(
        &self,
        root: String,
        path: String,
        version: String,
    ) -> Result<FileVersion, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::restore(&c, root, path, version).await }).await
    }
    async fn checkpoint(&self, root: String, name: String) -> Result<Checkpoint, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::checkpoint(&c, root, name).await }).await
    }
    async fn checkpoints(&self, root: String) -> Result<Vec<Checkpoint>, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::checkpoints(&c, root).await }).await
    }
    async fn assets(&self, root: String) -> Result<Vec<Asset>, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::assets(&c, root).await }).await
    }
    async fn search(&self, query: String) -> Result<Vec<Hit>, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::search(&c, query).await }).await
    }
    async fn peaks(&self, root: String, path: String) -> Result<Peaks, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::peaks(&c, root, path).await }).await
    }
    async fn restore_checkpoint(
        &self,
        root: String,
        checkpoint: String,
    ) -> Result<u32, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::restore_checkpoint(&c, root, checkpoint).await }).await
    }
    async fn drive(&self) -> Result<DriveStatus, AgentError> {
        let c = self.0.clone();
        on_runtime(async move { Files::drive(&c).await }).await
    }
}
