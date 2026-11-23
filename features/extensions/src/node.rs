//! An extension's server-side part on the org's node (#583, #710 H16).
//!
//! The node does sync, access control and only what truly needs a
//! server. A **node action** is the one exception for extension code: an
//! `[[actions]]` entry with `runs = "node"` that holds at least one of the
//! extension's `[[secrets]]`, a secret the person's devices must never
//! have (a Plaid token exchange holding the client secret, #370):
//!
//! ```toml
//! [capabilities]
//! hosts = ["sandbox.plaid.com"]
//!
//! [[secrets]]
//! id = "client_secret"
//! title = "Plaid secret"
//! hosts = ["sandbox.plaid.com"]
//!
//! [[actions]]
//! id = "LEDGER_EXCHANGE"
//! name = "Link a bank"
//! script = "node/exchange.luau"
//! runs = "node"
//! secrets = ["client_secret"]
//! ```
//!
//! The rules (`Manifest::check`): only a `luau` extension's action, with a
//! script, holding at least one declared secret, and never a preset, a
//! toggle or a window opener.
//!
//! **One implementation.** [`NodeActions`] is one `#[architect::rpc]`
//! service. [`NodeHost`] (feature `node`) is its one implementation: the
//! node loads each installed extension's node part
//! (`Manifest::node_part`: its node actions, their secrets, its hosts) on
//! a [`Host`] of its own, and serves it on the org lane
//! (`/org/{slug}/vox`) behind the org's permissions gate (its table: [`PERMITS`]).
//! Every client, the desktop, iOS and the browser, holds the generated
//! `NodeActionsClient`. Its own host registers a node action as a
//! forwarder to it ([`NodeLane`], `Host::with_node`), so ⌘K, the CLI, MCP
//! and a script run it as any other action.
//!
//! **What the node grants a node part.** Its node actions run under the
//! same capabilities model as the app's, narrowed to what a server may
//! do: `fetch` to its declared `capabilities.hosts`, each redirect
//! checked, and its secrets put into a request's headers by the host
//! (`{{secret:client_secret}}`), never readable by the script. No
//! documents, no interface, no ports, no events. The deployment that
//! names the extension (pinned version, hash and signature, through Nix)
//! is the node's consent.
//!
//! **Secrets are insert-only.** A node's secrets come from its operator
//! (a folder of files, `<dir>/<ext id>/<secret id>`, as sops-nix writes
//! them), or, on a node with no such folder, from an org admin through
//! [`NodeActions::set_secret`]. Nothing reads a value back: the service
//! says which are set, never what they are.

// The router that serves it asks for `Send` futures itself.
#![allow(async_fn_in_trait)]

#[cfg(feature = "vox")]
use facet::Facet;

/// One node action a node runs.
#[cfg(feature = "vox")]
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct NodeAction {
    /// Its extension's id.
    pub ext: String,
    /// Its extension's version.
    pub version: String,
    /// The action's id (`LEDGER_EXCHANGE`).
    pub id: String,
    pub name: String,
    pub description: String,
    /// The secrets it holds that the node has no value for yet: it is
    /// refused until they are set.
    pub missing: Vec<String>,
}

/// One of an extension's node secrets: whether it is set, never its
/// value.
#[cfg(feature = "vox")]
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct SecretState {
    pub id: String,
    pub title: String,
    pub set: bool,
}

/// Why the node did not run a node action.
#[cfg(feature = "vox")]
#[architect::error]
#[derive(Eq)]
pub enum NodeFault {
    /// The call carries no identity to act for.
    #[error("no identity: sign in")]
    Unauthenticated,
    /// The node runs no node action of that id for that extension.
    #[error("this node runs no node action {action} of {ext}")]
    NoAction { ext: String, action: String },
    /// A secret it holds has no value on this node.
    #[error("{ext} needs its secret {secret} on this node: its operator sets it")]
    NoSecret { ext: String, secret: String },
    /// The node's secrets are its operator's (a folder of files): set
    /// them in its deployment, not through the lane.
    #[error("this node's secrets are set by its deployment, not here")]
    OperatorSecrets,
    /// What was sent is not what it takes (JSON arguments, an id).
    #[error("{0}")]
    Invalid(String),
    /// The action ran and failed, or was refused a capability.
    #[error("{0}")]
    Failed(String),
}

/// What a client asks the org's node to run (#583): an extension's node
/// actions, served on the org lane.
#[cfg(feature = "vox")]
#[architect::rpc]
pub trait NodeActions {
    /// The node actions this node runs, each with the secrets it still
    /// lacks.
    async fn list(&self) -> Result<Vec<NodeAction>, NodeFault>;

    /// Runs node action `action` of extension `ext` with JSON `args`, for
    /// the caller: its JSON result.
    async fn run(&self, ext: String, action: String, args: String) -> Result<String, NodeFault>;

    /// Extension `ext`'s node secrets, and which are set.
    async fn secrets(&self, ext: String) -> Result<Vec<SecretState>, NodeFault>;

    /// Sets extension `ext`'s node secret `secret` (insert-only: no call
    /// reads it back). Refused on a node whose operator sets them
    /// ([`NodeFault::OperatorSecrets`]).
    async fn set_secret(&self, ext: String, secret: String, value: String)
    -> Result<(), NodeFault>;
}

/// The service [`PERMITS`] is for.
#[cfg(feature = "vox")]
pub const SERVICE: &str = "NodeActions";

/// One method's permit: what it needs (`action` on `resource`) and whether
/// a call is audited. A permissions gate builds its own rule from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Permit {
    pub method: &'static str,
    pub action: &'static str,
    pub resource: &'static str,
    pub audited: bool,
}

/// The gate's table for [`NodeActions`] on the org lane: any member of
/// the org lists them; running one is a member's (`write`); a secret is
/// an admin's.
#[cfg(feature = "vox")]
pub const PERMITS: &[Permit] = &[
    Permit {
        method: "list",
        action: "read",
        resource: "extensions/**",
        audited: false,
    },
    Permit {
        method: "run",
        action: "write",
        resource: "extensions/**",
        audited: true,
    },
    Permit {
        method: "secrets",
        action: "admin",
        resource: "extensions/**",
        audited: false,
    },
    Permit {
        method: "set_secret",
        action: "admin",
        resource: "extensions/**",
        audited: true,
    },
];

/// The future a [`NodeLane`] answers with: `Send` natively, not on wasm.
#[cfg(all(feature = "host", not(target_arch = "wasm32")))]
pub type NodeFuture = futures::future::BoxFuture<'static, Result<String, String>>;
/// The future a [`NodeLane`] answers with: `Send` natively, not on wasm.
#[cfg(all(feature = "host", target_arch = "wasm32"))]
pub type NodeFuture = futures::future::LocalBoxFuture<'static, Result<String, String>>;

/// How an app's host reaches its org's node to run a node action
/// (`Host::with_node`): the generated `NodeActionsClient` on the org
/// lane, or anything that answers as one (a test's in-process node).
#[cfg(feature = "host")]
pub trait NodeLane: Send + Sync {
    /// Runs `ext`'s node action `action` with JSON `args`; its JSON
    /// result, or why not.
    fn run(&self, ext: &str, action: &str, args: String) -> NodeFuture;
}

#[cfg(all(feature = "host", feature = "vox", not(target_arch = "wasm32")))]
impl NodeLane for NodeActionsClient {
    fn run(&self, ext: &str, action: &str, args: String) -> NodeFuture {
        let (client, ext, action) = (self.clone(), ext.to_owned(), action.to_owned());
        Box::pin(async move {
            client
                .run(ext, action, args)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

/// In a browser vox's client holds `Rc`s and the page has one thread:
/// the lane is the client in a `SendWrapper`, so the host stays `Send`.
#[cfg(all(feature = "host", feature = "vox", target_arch = "wasm32"))]
impl NodeLane for send_wrapper::SendWrapper<NodeActionsClient> {
    fn run(&self, ext: &str, action: &str, args: String) -> NodeFuture {
        let (client, ext, action) = ((**self).clone(), ext.to_owned(), action.to_owned());
        Box::pin(async move {
            client
                .run(ext, action, args)
                .await
                .map_err(|e| e.to_string())
        })
    }
}

#[cfg(feature = "node")]
pub use serve::NodeHost;

#[cfg(feature = "node")]
mod serve {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex, PoisonError};

    use architect::action::{ActionCaller, ActionError, ActionList};
    use architect_settings::Registry;

    use super::{NodeAction, NodeActions, NodeFault, SecretState};
    use crate::catalog::Origin;
    use crate::host::{Host, LoadError, SharedSections};
    use crate::manifest::Manifest;
    use crate::net::Secrets;
    use crate::package::Package;

    /// The node's side of [`NodeActions`]: the one implementation. Holds
    /// the node parts of the extensions it was given, on a [`Host`] of
    /// its own. Cheap to clone.
    #[derive(Clone)]
    pub struct NodeHost {
        host: Host,
        list: ActionList,
        /// What it holds, by extension id: each node part's manifest.
        held: Arc<Mutex<BTreeMap<String, Manifest>>>,
        /// The folder its operator's secrets are read from, when they are
        /// its operator's: then nothing on the lane sets one.
        operator: Option<PathBuf>,
    }

    impl std::fmt::Debug for NodeHost {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("NodeHost")
                .field("held", &self.ids())
                .field("operator", &self.operator)
                .finish_non_exhaustive()
        }
    }

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Why a folder's extension was not taken.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Skipped {
        /// Its package does not read.
        Package(String),
        /// It has no node action: nothing of it runs on the node.
        NoNodePart,
        /// Its node part did not load.
        Load(String),
    }

    impl NodeHost {
        /// A node host whose secrets an org admin sets through the lane,
        /// kept in `file` (`0600`), or in memory with `None`.
        #[must_use]
        pub fn new(secrets: Option<PathBuf>) -> Self {
            let secrets = secrets.map_or_else(Secrets::memory, Secrets::open);
            Self::with(secrets, None)
        }

        /// A node host whose secrets are its operator's: each file
        /// `<dir>/<ext id>/<secret id>` (its text, trimmed of a final
        /// newline), read as each extension loads. Nothing on the lane
        /// sets one.
        #[must_use]
        pub fn with_operator_secrets(dir: impl Into<PathBuf>) -> Self {
            Self::with(Secrets::memory(), Some(dir.into()))
        }

        fn with(secrets: Secrets, operator: Option<PathBuf>) -> Self {
            let list = ActionList::new();
            let host = Host::new(list.clone(), SharedSections::new(Registry::new()))
                .with_secrets(secrets)
                .serving_node();
            Self {
                host,
                list,
                held: Arc::default(),
                operator,
            }
        }

        /// The ids of the extensions it holds a node part of.
        #[must_use]
        pub fn ids(&self) -> Vec<String> {
            lock(&self.held).keys().cloned().collect()
        }

        /// The host its node parts are loaded on (its secrets, its logs).
        #[must_use]
        pub const fn host(&self) -> &Host {
            &self.host
        }

        /// Loads `package`'s node part, in place of any it held. Whether
        /// it had one.
        ///
        /// # Errors
        ///
        /// Its node part does not load (a script that is not Luau).
        pub fn load(&self, package: &Package) -> Result<bool, LoadError> {
            // Its note file types, whether or not it has a node part: the
            // node's vaults read them as notes.
            for t in &package.manifest.contributes.file_types {
                t.register();
            }
            let Some(part) = package.manifest.node_part() else {
                return Ok(false);
            };
            if let Some(dir) = &self.operator {
                self.read_operator_secrets(dir, &part);
            }
            let node_package = Package {
                manifest: part.clone(),
                files: package.files.clone(),
            };
            self.host.load(&node_package, Origin::Installed)?;
            lock(&self.held).insert(part.id.clone(), part);
            Ok(true)
        }

        /// Loads the node part of each package in `dir` (one folder an
        /// id, `<id>/extension.toml`, as `Installs` keeps them): the ids
        /// loaded, and each folder skipped with why. A folder with no
        /// node action is skipped, never an error.
        #[must_use]
        pub fn load_folder(&self, dir: &Path) -> (Vec<String>, Vec<(PathBuf, Skipped)>) {
            let mut loaded = Vec::new();
            let mut skipped = Vec::new();
            let Ok(entries) = std::fs::read_dir(dir) else {
                return (loaded, skipped);
            };
            let mut folders: Vec<PathBuf> = entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.join(crate::manifest::MANIFEST).is_file())
                .collect();
            folders.sort();
            for folder in folders {
                match Package::read(&folder) {
                    Err(e) => skipped.push((folder, Skipped::Package(e.to_string()))),
                    Ok(package) => match self.load(&package) {
                        Ok(true) => loaded.push(package.manifest.id),
                        Ok(false) => skipped.push((folder, Skipped::NoNodePart)),
                        Err(e) => skipped.push((folder, Skipped::Load(e.to_string()))),
                    },
                }
            }
            (loaded, skipped)
        }

        fn read_operator_secrets(&self, dir: &Path, part: &Manifest) {
            let secrets = self.host.secrets();
            for s in &part.secrets {
                let file = dir.join(&part.id).join(&s.id);
                match std::fs::read_to_string(&file) {
                    Ok(text) => {
                        let value = text.strip_suffix('\n').unwrap_or(&text);
                        let value = value.strip_suffix('\r').unwrap_or(value);
                        let _ = secrets.set(&part.id, &s.id, value);
                    }
                    Err(_) => {
                        let _ = secrets.forget(&part.id, &s.id);
                    }
                }
            }
        }

        fn missing(&self, part: &Manifest, action: &str) -> Vec<String> {
            let secrets = self.host.secrets();
            part.actions
                .iter()
                .filter(|a| a.id == action)
                .flat_map(|a| a.secrets.iter())
                .filter(|s| !secrets.is_set(&part.id, s))
                .cloned()
                .collect()
        }

        /// [`NodeActions::run`] as `caller` (the person the org lane's gate
        /// resolved, or a test's).
        ///
        /// # Errors
        ///
        /// As [`NodeFault`] says.
        pub async fn run_as(
            &self,
            caller: ActionCaller,
            ext: &str,
            action: &str,
            args: &str,
        ) -> Result<String, NodeFault> {
            let part = lock(&self.held).get(ext).cloned();
            let no_action = || NodeFault::NoAction {
                ext: ext.to_owned(),
                action: action.to_owned(),
            };
            let part = part.ok_or_else(no_action)?;
            if !part.actions.iter().any(|a| a.id == action) {
                return Err(no_action());
            }
            if let Some(secret) = self.missing(&part, action).into_iter().next() {
                return Err(NodeFault::NoSecret {
                    ext: ext.to_owned(),
                    secret,
                });
            }
            let args = if args.trim().is_empty() { "{}" } else { args };
            serde_json::from_str::<serde_json::Value>(args)
                .map_err(|e| NodeFault::Invalid(format!("its arguments are not JSON: {e}")))?;
            let out = self
                .list
                .run_json_async_with(&caller, action, args.as_bytes())
                .await
                .map_err(|e| match e {
                    ActionError::Failed { message, .. } => NodeFault::Failed(message),
                    other => NodeFault::Failed(other.to_string()),
                })?;
            String::from_utf8(out).map_err(|e| NodeFault::Failed(e.to_string()))
        }
    }

    fn caller() -> Result<ActionCaller, NodeFault> {
        architect::permissions_gate::caller()
            .map(ActionCaller::new)
            .ok_or(NodeFault::Unauthenticated)
    }

    impl NodeActions for NodeHost {
        async fn list(&self) -> Result<Vec<NodeAction>, NodeFault> {
            caller()?;
            let held = lock(&self.held).clone();
            Ok(held
                .values()
                .flat_map(|part| {
                    part.actions.iter().map(move |a| NodeAction {
                        ext: part.id.clone(),
                        version: part.version.to_string(),
                        id: a.id.clone(),
                        name: a.name.clone(),
                        description: a.description.clone(),
                        missing: self.missing(part, &a.id),
                    })
                })
                .collect())
        }

        async fn run(
            &self,
            ext: String,
            action: String,
            args: String,
        ) -> Result<String, NodeFault> {
            let caller = caller()?;
            let span = tracing::info_span!(
                "ext.node.run",
                ext.id = %ext,
                ext.action = %action,
                ext.outcome = tracing::field::Empty,
            );
            let result = {
                let _entered = span.enter();
                self.run_as(caller, &ext, &action, &args)
            }
            .await;
            span.record("ext.outcome", if result.is_ok() { "ok" } else { "failed" });
            result
        }

        async fn secrets(&self, ext: String) -> Result<Vec<SecretState>, NodeFault> {
            caller()?;
            let part = lock(&self.held).get(&ext).cloned();
            let part = part.ok_or_else(|| NodeFault::NoAction {
                ext: ext.clone(),
                action: String::new(),
            })?;
            let secrets = self.host.secrets();
            Ok(part
                .secrets
                .iter()
                .map(|s| SecretState {
                    id: s.id.clone(),
                    title: s.title.clone(),
                    set: secrets.is_set(&ext, &s.id),
                })
                .collect())
        }

        async fn set_secret(
            &self,
            ext: String,
            secret: String,
            value: String,
        ) -> Result<(), NodeFault> {
            caller()?;
            if self.operator.is_some() {
                return Err(NodeFault::OperatorSecrets);
            }
            let part = lock(&self.held).get(&ext).cloned();
            let declared = part.is_some_and(|p| p.secrets.iter().any(|s| s.id == secret));
            if !declared {
                return Err(NodeFault::Invalid(format!(
                    "{ext} holds no node secret {secret}"
                )));
            }
            if value.is_empty() {
                return Err(NodeFault::Invalid("an empty secret".to_owned()));
            }
            self.host
                .secrets()
                .set(&ext, &secret, &value)
                .map_err(NodeFault::Failed)
        }
    }
}

#[cfg(feature = "node")]
pub use serve::Skipped;
