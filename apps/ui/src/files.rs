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
use architect_files::roots::{Asset, Checkpoint, FileEntry, FileVersion, Hit, RootStatus};

/// Where the agent listens: `http://127.0.0.1:8090`.
pub const AGENT: &str = "ARCHITECT_FILES_AGENT";

/// `FILES_*` on `list`, when the device has an agent.
pub fn register(list: &ActionList) {
    let Ok(base) = std::env::var(AGENT) else {
        return;
    };
    let agent = Agent(FilesHttpClient::at(base.trim_end_matches('/')));
    if let Err(why) = files_actions::register(agent, list) {
        tracing::warn!(?why, "FILES_* are not on the list");
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
}
