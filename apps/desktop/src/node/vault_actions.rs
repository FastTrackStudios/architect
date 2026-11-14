//! The org's vaults' actions on the org lane (#495): `VaultActions`
//! (`vault_proto::vault_actions`), which a signed-in member's web app runs
//! to reach a vault's `VAULT_*`, `LINKS_*`, `TAGS_*` and `BASES_*`
//! actions. The node runs the desktop app's own actions over each vault's
//! folder (`vault::actions::OpenVault`): one engine, the browser keeps no
//! copy.
//!
//! - **One Action List per vault**, opened on first use and kept. A write
//!   lands in the vault's folder, which the node watches, so it reaches
//!   every peer as an edit made on disk does.
//! - **Fresh enough.** Peers' edits reach the folder through the node's
//!   backend, not through the open vault, so it is read again before a
//!   call once its last read is older than [`FRESH`].
//! - **Permissions.** The lane's gate decides each call by method:
//!   `actions` and `run_read` need `read` on the vaults, `run_write`
//!   needs `write` (audited). `run_read` runs only a read-only action.
//!   The action runs as the member the gate admitted, so the list's own
//!   authorizer sees who it is.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use architect::action::{ActionCaller, ActionList, Effect};
use vault::actions::OpenVault;
use vault_live::Backend;
use vault_proto::vault_actions::{VaultActionInfo, VaultActions, VaultActionsError};

/// How old an open vault's read may be before a call reads it again.
pub const FRESH: Duration = Duration::from_secs(1);

/// One vault's open copy and its actions.
struct Open {
    vault: OpenVault,
    list: Arc<ActionList>,
    read_at: Instant,
}

/// The org lane's `VaultActions`. Cheap to clone; clones share the open
/// vaults.
#[derive(Clone)]
pub struct VaultActionsState {
    backend: Backend,
    open: Arc<Mutex<BTreeMap<String, Open>>>,
}

impl VaultActionsState {
    #[must_use]
    pub fn new(backend: Backend) -> Self {
        Self {
            backend,
            open: Arc::default(),
        }
    }

    /// Vault `id`'s actions, its read fresh.
    fn list(&self, id: &str) -> Result<Arc<ActionList>, VaultActionsError> {
        let mut open = architect::lock(&self.open);
        if let Some(o) = open.get_mut(id) {
            if o.read_at.elapsed() > FRESH {
                o.vault
                    .reload()
                    .map_err(|e| VaultActionsError::Failed(format!("{id}: {e}")))?;
                o.read_at = Instant::now();
            }
            return Ok(Arc::clone(&o.list));
        }
        let root = self
            .backend
            .root(id)
            .map_err(|_| VaultActionsError::NotFound(format!("vault {id}")))?;
        let vault = OpenVault::open(&root)
            .map_err(|e| VaultActionsError::Failed(format!("{id}: {e}")))?;
        let list = Arc::new(ActionList::new());
        register(&list, &vault).map_err(|e| VaultActionsError::Failed(e.to_string()))?;
        open.insert(
            id.to_owned(),
            Open {
                vault,
                list: Arc::clone(&list),
                read_at: Instant::now(),
            },
        );
        Ok(list)
    }

    /// Runs `id` on vault `vault` as the gate's caller; `read`: only a
    /// read-only action.
    async fn run(
        &self,
        vault: &str,
        id: &str,
        args: &str,
        read: bool,
    ) -> Result<String, VaultActionsError> {
        let who = architect::permissions_gate::caller().ok_or_else(|| {
            VaultActionsError::Failed("no caller: the lane admits members only".to_owned())
        })?;
        let list = self.list(vault)?;
        let info = list
            .info(id)
            .ok_or_else(|| VaultActionsError::NotFound(format!("action {id}")))?;
        if read && info.effect != Effect::ReadOnly {
            return Err(VaultActionsError::NotRead(id.to_owned()));
        }
        let caller = ActionCaller::new(who);
        let out = list
            .run_json_async_with(&caller, id, args.as_bytes())
            .await
            .map_err(|e| VaultActionsError::Failed(e.to_string()))?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}

/// The actions a vault answers here: the desktop app's own over its vault
/// (`apps/ui/src/notes.rs` `register`).
fn register(list: &Arc<ActionList>, vault: &OpenVault) -> Result<(), architect::action::ActionError> {
    vault::actions::register(list, vault)?;
    links::actions::register(list, vault)?;
    tag::actions::register(list, vault)?;
    architect_bases::actions::register(list, vault, &architect_bases::model::ViewTypes::default())
}

const fn effect_name(e: Effect) -> &'static str {
    match e {
        Effect::ReadOnly => "read_only",
        Effect::Mutating => "mutating",
        Effect::Destructive => "destructive",
    }
}

impl VaultActions for VaultActionsState {
    async fn actions(&self, vault: String) -> Result<Vec<VaultActionInfo>, VaultActionsError> {
        let list = self.list(&vault)?;
        Ok(list
            .infos()
            .into_iter()
            .map(|i| VaultActionInfo {
                id: i.id,
                name: i.display_name,
                description: i.description,
                group: i.group,
                effect: effect_name(i.effect).to_owned(),
                params: i.params_schema,
            })
            .collect())
    }

    async fn run_read(
        &self,
        vault: String,
        id: String,
        args: String,
    ) -> Result<String, VaultActionsError> {
        self.run(&vault, &id, &args, true).await
    }

    async fn run_write(
        &self,
        vault: String,
        id: String,
        args: String,
    ) -> Result<String, VaultActionsError> {
        self.run(&vault, &id, &args, false).await
    }
}

/// [`VaultActions`]'s declaration, its generated server with it.
pub mod service {
    pub use vault_proto::vault_actions::*;
}
