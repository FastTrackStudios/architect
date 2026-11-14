//! A node's vault actions, as a service (#495): what a signed-in client
//! (the web app, `apps/web`) runs over the node's org lane
//! (`/org/{slug}/vox`, the session as the bearer) to reach a vault's
//! `VAULT_*` actions, the same ones the
//! desktop app runs on its own vault (queries, `BASES_*`, are the vault
//! query service's, #272). The node keeps one engine; the browser keeps
//! no copy of it.
//!
//! A read and a write are separate calls so the lane's gate decides each
//! by the member's permission (`read` or `write` on the vault):
//! [`VaultActions::run_read`] runs only a read-only action.

/// An action a vault answers, as the node lists it.
#[architect::wire]
#[derive(Eq)]
pub struct VaultActionInfo {
    /// Its id (`VAULT_READ`).
    pub id: String,
    pub name: String,
    pub description: String,
    pub group: String,
    /// `read_only`, `mutating` or `destructive`.
    pub effect: String,
    /// Its parameters' JSON Schema, when it takes any.
    pub params: Option<String>,
}

/// Why the node refused.
#[architect::error]
#[derive(Eq)]
pub enum VaultActionsError {
    #[error("no such vault or action: {0}")]
    #[architect(http_status = 404)]
    NotFound(String),
    /// [`VaultActions::run_read`] was asked for an action that changes the
    /// vault.
    #[error("{0} changes the vault: run it as a write")]
    #[architect(http_status = 403)]
    NotRead(String),
    /// The action ran and failed, or refused its arguments.
    #[error("{0}")]
    #[architect(http_status = 400)]
    Failed(String),
}

/// A node's vaults' actions.
#[architect::service]
#[allow(async_fn_in_trait)]
pub trait VaultActions {
    /// The actions vault `vault` answers.
    async fn actions(&self, vault: String) -> Result<Vec<VaultActionInfo>, VaultActionsError>;
    /// Runs read-only action `id` on vault `vault` with `args` (JSON): its
    /// answer as JSON.
    async fn run_read(
        &self,
        vault: String,
        id: String,
        args: String,
    ) -> Result<String, VaultActionsError>;
    /// Runs action `id` on vault `vault` with `args` (JSON), whatever its
    /// effect: its answer as JSON.
    async fn run_write(
        &self,
        vault: String,
        id: String,
        args: String,
    ) -> Result<String, VaultActionsError>;
}
pub use prelude::*;
