//! What the node answers, declared once each (`#[architect::service]`).
//!
//! - [`Node`]: what this node is (`status`). Its whole face is public, so
//!   it is served on every wire: same-origin vox (`/vox`), the HTTP face
//!   (`POST /node/status`) and the Action List (`NODE_STATUS`, over MCP).
//! - [`Notes`]: the notes of the vaults it holds, read and written. Only
//!   on the Action List, so only over `/mcp`, which needs its bearer: an
//!   agent writes a note on the always-on node and every device that
//!   syncs with it gets it.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use architect_permissions::Principal;
use vault::actions::query::DiskSource;

use architect_devices::Admission;
use vault::query::Built;
use vault_live::Backend;
use vault_proto::index_sync::{IndexBytes, IndexError, VaultIndexSync};
use vault_proto::{IfMatch, VaultSync as _, VaultSyncError};

use super::catalog::VaultCatalog;

/// Why the node refused.
#[architect::error]
#[derive(Eq)]
pub enum NodeError {
    #[error("no such vault or note: {0}")]
    #[architect(http_status = 404)]
    NotFound(String),
    #[error("{0}")]
    #[architect(http_status = 400)]
    Refused(String),
}

impl From<VaultSyncError> for NodeError {
    fn from(e: VaultSyncError) -> Self {
        match e {
            VaultSyncError::NotFound => Self::NotFound("the vault or the note".to_owned()),
            other => Self::Refused(other.to_string()),
        }
    }
}

/// What a node is: its name and endpoint, what it holds and who it knows.
#[architect::wire]
#[derive(Eq)]
pub struct NodeStatus {
    /// Its name on the account.
    pub name: String,
    /// `CARGO_PKG_VERSION` of the build.
    pub version: String,
    /// Its iroh endpoint id: how the others dial it.
    pub endpoint: String,
    /// The org the account's devices share, once the pass found one.
    pub org: Option<String>,
    /// The org whose vaults it serves on its org lane, `/org/{slug}/vox`
    /// (`Config::host_org`): what a signed-in web app opens.
    pub hosts: Option<String>,
    /// Every org it serves so, on a node that hosts several (#538): the
    /// first is [`Self::hosts`].
    #[facet(default)]
    pub hosted: Vec<String>,
    /// The same, each with the issuer's id for it: what a device that
    /// holds an org's files with no session of its own (a storage server
    /// authorised by its key) names the org by.
    #[facet(default)]
    pub hosted_orgs: Vec<HostedOrg>,
    /// Whether its last account pass went through.
    pub signed_in: bool,
    /// How many vaults it holds, and their notes.
    pub vaults: u32,
    pub notes: u32,
    /// Where its Sites are mounted.
    pub sites: Vec<String>,
    /// The devices it lets in.
    pub devices: u32,
    /// Its blob store against its budget, when it runs the files engine.
    pub files_store: Option<FilesStore>,
    /// The relays its iroh endpoint is reachable through: none with
    /// `ARCHITECT_IROH=local`.
    pub relays: Vec<String>,
    /// The addresses its iroh endpoint is bound to (`127.0.0.1:…` with
    /// `ARCHITECT_IROH=local`).
    pub iroh_addrs: Vec<String>,
    /// What its share links' layer rules need the owner to know (#528): a
    /// kept-rules file set aside, a manifest that does not read, rules a
    /// reinstall lowered; one line each, naming nothing of the vault.
    /// Details: `SHARE_LAYERS_KEPT`.
    pub share_layers: Vec<String>,
}

/// An org a node hosts ([`NodeStatus::hosted_orgs`]).
#[architect::wire]
#[derive(Eq)]
pub struct HostedOrg {
    pub slug: String,
    /// The issuer's id for it.
    pub id: String,
}

/// The files engine's blob store against its budget (#525).
#[architect::wire]
#[derive(Eq)]
pub struct FilesStore {
    /// Bytes in the store, and promised to writes in flight.
    pub used: u64,
    pub reserved: u64,
    /// Its limit (`files_store_max`).
    pub max: u64,
    /// The floor kept free on its filesystem (`files_keep_free`), and
    /// that filesystem's free bytes and size, when it can be asked.
    pub keep_free: u64,
    pub fs_free: Option<u64>,
    pub fs_total: Option<u64>,
    /// Files waiting for room now, files larger than the whole limit
    /// (they arrive once it is raised), and writes refused for space
    /// since start.
    pub deferred: u32,
    pub too_large: u32,
    pub refused: u64,
    /// Replacements waiting because the versions they replace cannot be
    /// kept, and why ("cannot keep history: …": a read-only share, a
    /// `.architect` the node cannot write).
    pub no_history: u32,
    pub history_error: Option<String>,
    /// Replacements and deletes waiting because the kept versions are at
    /// their bound (`history_used` against `history_max`, or the tree
    /// filesystem's floor). Nothing frees history by itself: raise the
    /// bound or make room.
    pub history_full: u32,
    /// A holding node's kept versions, beside its tree: bytes, limit
    /// (`files_history_max`; none is `u64::MAX`) and the tree
    /// filesystem's free bytes. Not counted in `used`.
    pub history_used: Option<u64>,
    pub history_max: Option<u64>,
    pub tree_fs_free: Option<u64>,
    /// A holding node's fetches in flight or waiting to be written,
    /// beside its tree.
    pub incoming_used: Option<u64>,
    /// Files its holding rule leaves out (#904, `files_hold_paths`,
    /// `files_skip_paths`, `files_hold_max`): listed here, their bytes on
    /// the holders that keep them. Not waiting for anything.
    #[facet(default)]
    pub left_out: u32,
}

/// A note in a vault: `vault_proto::notes`' (the `Notes` contract lives
/// there, so a browser has its client).
pub use vault_proto::notes::NoteRef;

/// [`Node`]'s declaration: one service per module, as the generated
/// names (`http`, `prelude`) are per module.
pub mod node {
    use super::{NodeError, NodeStatus};

    /// What this node is.
    #[architect::service(actions)]
    pub trait Node {
        /// This node: its name, endpoint, org, vaults, Sites, devices, and
        /// its blob store against its budget.
        #[action(effect = read_only)]
        async fn status(&self) -> Result<NodeStatus, NodeError>;
    }

    pub use prelude::*;
}

/// [`Notes`]'s declaration: `vault_proto::notes`, its generated client,
/// server and actions with it.
pub mod notes {
    pub use vault_proto::notes::*;
}

/// [`VaultIndexSync`]'s declaration: `vault_proto::index_sync`.
pub mod index_sync {
    pub use vault_proto::index_sync::*;
}

pub use node::Node;
pub use notes::{Notes, NotesError};

/// What the account pass last said.
#[derive(Debug, Clone, Default)]
pub struct AccountView {
    pub org: Option<String>,
    pub signed_in: bool,
}

/// The node's state, as its services see it. Cheap to clone.
#[derive(Clone)]
pub struct NodeState {
    pub(crate) name: String,
    pub(crate) endpoint: String,
    /// The iroh endpoint itself: where it is bound and which relays it uses.
    pub(crate) iroh: Option<iroh::Endpoint>,
    pub(crate) backend: Backend,
    pub(crate) sites: Vec<String>,
    pub(crate) admission: Admission,
    pub(crate) account: Arc<Mutex<AccountView>>,
    pub(crate) files: Option<architect_files::sync::Engine>,
    /// Its share links' layer rules, for its status.
    pub(crate) layers: Option<super::share_layers::LayerRules>,
    /// The orgs it hosts on its org lanes, if any.
    pub(crate) hosted: Vec<String>,
    /// Whose each vault is: a call on an org's lane sees only its own.
    pub(crate) orgs: super::org::VaultOrgs,
    /// Each vault's index, for clients that do not hold it.
    pub(crate) indexed: Indexed,
}

/// What a caller may read: the org lane's (`OrgLane::reads`,
/// `OrgLane::read_key`), set when the node hosts an org.
#[derive(Clone)]
pub struct Access {
    /// Whether `(who, vault, path)` may be read.
    pub reads: Arc<dyn Fn(&Principal, &str, &str) -> bool + Send + Sync>,
    /// A fingerprint of what `who` may read in a vault (their role in
    /// its org): when it moves, their next `since` answers a snapshot.
    pub key: Arc<dyn Fn(&Principal, &str) -> u64 + Send + Sync>,
}

/// How long a vault's index stays held with no one asking for it: past
/// it, it is let go (a reader's next `since` is of another history, and
/// it takes a snapshot).
pub const INDEX_IDLE: std::time::Duration = std::time::Duration::from_mins(30);

/// One vault's index, built once however many ask at once.
type Slot = Arc<std::sync::OnceLock<Result<Arc<Built<DiskSource>>, String>>>;

/// Each vault's index, by id, for clients that do not hold the vault
/// ([`VaultIndexSync`]).
///
/// Each is built a note at a time (`vault_query::index::Builder`) from
/// the vault's folder (`vault::actions::query::DiskSource`), which holds
/// no note's text and reads only what changed. The node answers no
/// queries: a client runs them over its own copy of the index
/// (`vault_query::QueryEngine`). What a caller gets is what [`Access`]
/// lets it read.
#[derive(Clone, Default)]
pub struct Indexed {
    held: Arc<Mutex<HashMap<String, (Slot, std::time::Instant)>>>,
    access: Arc<Mutex<Option<Access>>>,
}

impl Indexed {
    /// Who may read what, from now on.
    pub fn set_access(&self, access: Access) {
        *architect::lock(&self.access) = Some(access);
    }

    fn access(&self) -> Option<Access> {
        architect::lock(&self.access).clone()
    }

    /// Vault `id`'s index at `root`: built the first time it is asked for
    /// (once, whoever else asks meanwhile); the vaults no one asked for
    /// in [`INDEX_IDLE`] let go.
    fn of(&self, id: &str, root: &std::path::Path) -> Result<Arc<Built<DiskSource>>, String> {
        let slot = {
            let mut held = architect::lock(&self.held);
            let now = std::time::Instant::now();
            held.retain(|v, (_, used)| v == id || now.duration_since(*used) < INDEX_IDLE);
            let (slot, used) = held
                .entry(id.to_owned())
                .or_insert_with(|| (Slot::default(), now));
            *used = now;
            let slot = Arc::clone(slot);
            drop(held);
            slot
        };
        // Built outside the map's lock: a large vault takes a while.
        slot.get_or_init(|| {
            let built = Arc::new(Built::new(DiskSource::open(root)));
            let _ = built.sync();
            Ok(built)
        })
        .clone()
    }
}

impl NodeState {
    fn catalog(&self) -> VaultCatalog {
        VaultCatalog::new(self.backend.clone())
    }

    /// Whether the node lets `endpoint` in.
    #[must_use]
    pub fn admits(&self, endpoint: &str) -> bool {
        self.admission.admits(endpoint)
    }

    /// Where vault `id` lives on this node.
    #[must_use]
    pub fn vault_root(&self, id: &str) -> Option<std::path::PathBuf> {
        self.backend.root(id).ok()
    }

    /// The status, without the wire.
    #[must_use]
    pub fn snapshot(&self) -> NodeStatus {
        let account = architect::lock(&self.account).clone();
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        NodeStatus {
            name: self.name.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            endpoint: self.endpoint.clone(),
            org: account.org,
            hosts: self.hosted.first().cloned(),
            hosted: self.hosted.clone(),
            hosted_orgs: self
                .orgs
                .orgs()
                .iter()
                .filter(|org| self.hosted.contains(&org.slug))
                .map(|org| HostedOrg {
                    slug: org.slug.clone(),
                    id: org.id.0.clone(),
                })
                .collect(),
            signed_in: account.signed_in,
            vaults: count(self.backend.vault_ids().len()),
            notes: count(self.catalog().notes().len()),
            sites: self.sites.clone(),
            devices: count(self.admission.list().len()),
            files_store: self.files.as_ref().and_then(|engine| {
                let usage = engine.store_usage()?;
                let history = engine.history_usage();
                let blocked = engine.history_blocked();
                Some(FilesStore {
                    used: usage.used,
                    reserved: usage.reserved,
                    max: usage.max,
                    keep_free: usage.keep_free,
                    fs_free: usage.space.map(|s| s.free),
                    fs_total: usage.space.map(|s| s.total),
                    deferred: engine.deferred(),
                    too_large: engine.too_large(),
                    refused: usage.refused,
                    no_history: blocked.0,
                    history_full: engine.history_full(),
                    history_error: blocked.1,
                    history_used: history.map(|h| h.used),
                    history_max: history.map(|h| h.max),
                    tree_fs_free: history.and_then(|h| h.space).map(|s| s.free),
                    incoming_used: engine.inbox_usage().map(|i| i.used),
                    left_out: engine.left_out(),
                })
            }),
            relays: self.iroh.as_ref().map_or_else(Vec::new, |e| {
                e.addr().relay_urls().map(ToString::to_string).collect()
            }),
            iroh_addrs: self.iroh.as_ref().map_or_else(Vec::new, |e| {
                e.bound_sockets().iter().map(ToString::to_string).collect()
            }),
            share_layers: self
                .layers
                .as_ref()
                .map(super::share_layers::LayerRules::status)
                .unwrap_or_default(),
        }
    }
}

impl Node for NodeState {
    async fn status(&self) -> Result<NodeStatus, NodeError> {
        Ok(self.snapshot())
    }
}

impl Notes for NodeState {
    async fn list(&self) -> Result<Vec<NoteRef>, NotesError> {
        Ok(self
            .catalog()
            .notes()
            .into_iter()
            .filter_map(|d| {
                d.name.split_once('/').map(|(v, p)| NoteRef {
                    vault: v.to_owned(),
                    path: p.to_owned(),
                })
            })
            // On an org's lane: that org's vaults only.
            .filter(|note| self.orgs.serves(&note.vault))
            .collect())
    }

    async fn read(&self, vault: String, path: String) -> Result<String, NotesError> {
        self.served(&vault)?;
        let bytes = self.backend.get_file(&vault, &path)?;
        Ok(String::from_utf8_lossy(&bytes.0).into_owned())
    }

    async fn write(&self, vault: String, path: String, text: String) -> Result<(), NotesError> {
        self.put_note(vault, path, text, IfMatch::Force).await
    }

    async fn create(&self, vault: String, path: String, text: String) -> Result<(), NotesError> {
        self.put_note(vault, path, text, IfMatch::CreateOnly).await
    }
}

impl NodeState {
    /// Vault `vault`, when the lane this call came in on serves it: on a
    /// node of several orgs another org's vault is not found.
    fn served(&self, vault: &str) -> Result<(), NotesError> {
        if self.orgs.serves(vault) {
            Ok(())
        } else {
            Err(NotesError::NotFound("the vault or the note".to_owned()))
        }
    }

    /// Runs `f` over vault `vault`'s index off the runtime's threads
    /// (opening a vault reads every note).
    async fn indexed<R: Send + 'static>(
        &self,
        vault: String,
        f: impl FnOnce(&Built<DiskSource>) -> R + Send + 'static,
    ) -> Result<R, IndexError> {
        let root = self
            .backend
            .root(&vault)
            .map_err(|_| IndexError::NotFound(vault.clone()))?;
        let indexed = self.indexed.clone();
        tokio::task::spawn_blocking(move || {
            let built = indexed.of(&vault, &root).map_err(IndexError::Refused)?;
            Ok(f(&built))
        })
        .await
        .map_err(|e| IndexError::Refused(e.to_string()))?
    }
}

impl VaultIndexSync for NodeState {
    async fn snapshot(&self, vault: String) -> Result<IndexBytes, IndexError> {
        let (reads, key) = self.reads(&vault)?;
        self.indexed(vault, move |built| snapshot_for(built, &*reads, key))
            .await
    }

    async fn since(
        &self,
        vault: String,
        epoch: u64,
        access: u64,
        from: u64,
    ) -> Result<Option<IndexBytes>, IndexError> {
        let (reads, key) = self.reads(&vault)?;
        self.indexed(vault, move |built| {
            since_for(built, &*reads, key, epoch, access, from)
        })
        .await
    }
}

/// The index of `built` as a reader that `reads` may read it, whose
/// rights' fingerprint is `key`.
pub fn snapshot_for<S: vault::query::NoteSource>(
    built: &Built<S>,
    reads: &dyn Fn(&str) -> bool,
    key: u64,
) -> IndexBytes {
    let index = built.index();
    IndexBytes {
        epoch: index.epoch,
        access: key,
        revision: index.revision,
        bytes: index.filtered(reads).to_bytes(),
    }
}

/// What changed in `built`'s index for a reader that `reads` may read
/// (its rights' fingerprint `key` now), since revision `from` of history
/// `epoch` read with access `access`; `None`: take a snapshot.
pub fn since_for<S: vault::query::NoteSource>(
    built: &Built<S>,
    reads: &dyn Fn(&str) -> bool,
    key: u64,
    epoch: u64,
    access: u64,
    from: u64,
) -> Option<IndexBytes> {
    // What it may read changed: it starts again from a snapshot.
    if key != access {
        return None;
    }
    let delta = built.since(epoch, from)?;
    let now = key;
    let mine = delta.filtered(reads);
    // Nothing it may read changed: it stays where it was, so it cannot
    // count the edits to notes it cannot read.
    let mine = if mine.is_empty() {
        vault::query::IndexDelta::none(epoch, from)
    } else {
        mine
    };
    Some(IndexBytes {
        epoch,
        access: now,
        revision: mine.to,
        bytes: mine.to_bytes(),
    })
}

impl NodeState {
    /// What the caller of this call may read in `vault`: a check of each
    /// note's path, through the org lane's rules ([`Access`]).
    #[allow(clippy::type_complexity)]
    fn reads(
        &self,
        vault: &str,
    ) -> Result<(Arc<dyn Fn(&str) -> bool + Send + Sync>, u64), IndexError> {
        let who = architect::permissions_gate::caller()
            .ok_or_else(|| IndexError::Refused("no caller: the index is a member's".to_owned()))?;
        if !self.orgs.serves(vault) {
            return Err(IndexError::NotFound(vault.to_owned()));
        }
        let access = self
            .indexed
            .access()
            .ok_or_else(|| IndexError::Refused("this node hosts no org".to_owned()))?;
        let key = (access.key)(&who, vault);
        let vault = vault.to_owned();
        Ok((
            Arc::new(move |path: &str| (access.reads)(&who, &vault, path)),
            key,
        ))
    }
}

impl NodeState {
    /// `text` into the note at `path`, as `if_match` allows: a note there
    /// already refuses [`IfMatch::CreateOnly`] ([`NotesError::Exists`]).
    async fn put_note(
        &self,
        vault: String,
        path: String,
        text: String,
        if_match: IfMatch,
    ) -> Result<(), NotesError> {
        self.served(&vault)?;
        // Registered first, so an open document merges the write as a
        // change rather than the next open finding it.
        self.backend.register_collab(&vault, &path)?;
        // `put_file` announces the write under a blocking lock: off the
        // runtime's worker threads. The existence check is under that lock
        // too, so two creates of one path cannot both succeed.
        let backend = self.backend.clone();
        let at = format!("{vault}/{path}");
        tokio::task::spawn_blocking(move || {
            backend.put_file(&vault, &path, text.into_bytes(), if_match)
        })
        .await
        .map_err(|e| NotesError::Refused(e.to_string()))?
        .map_err(|e| match e {
            VaultSyncError::Conflict { .. } => NotesError::Exists(at),
            other => other.into(),
        })?;
        Ok(())
    }
}

/// The public wires: [`Node`] alone.
impl architect::Services for NodeState {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![node::NodeService]
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use vault::query::{IndexDelta, VaultIndex};

    use super::*;

    fn write(dir: &std::path::Path, path: &str, text: &str) {
        let abs = dir.join(path);
        std::fs::create_dir_all(abs.parent().unwrap()).unwrap();
        std::fs::write(abs, text).unwrap();
    }

    fn settled(built: &Built<DiskSource>, done: impl Fn(&VaultIndex) -> bool) {
        for _ in 0..200 {
            if done(&built.index()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        panic!("the index did not catch up");
    }

    /// Two readers of one vault: one may read everything, one only
    /// `Public/`. Each gets only its notes, never a hidden path's name, no
    /// sign of edits it cannot see, and a snapshot when what it may read
    /// changes.
    #[test]
    fn each_reader_gets_only_what_it_may_read() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Public/A.md", "# A\n");
        write(dir.path(), "Private/B.md", "# B\n");
        let built = Built::new(DiskSource::open(dir.path()));
        let everything = |_: &str| true;
        let public = |p: &str| p.starts_with("Public/");

        let all = snapshot_for(&built, &everything, 1);
        let mine = snapshot_for(&built, &public, 2);
        assert_eq!(VaultIndex::from_bytes(&all.bytes).unwrap().entries.len(), 2);
        let mine_index = VaultIndex::from_bytes(&mine.bytes).unwrap();
        assert_eq!(
            mine_index.entries.keys().map(|k| &**k).collect::<Vec<_>>(),
            ["Public/A.md"]
        );
        assert!(!String::from_utf8_lossy(&mine.bytes).contains("Private"));

        // A private edit: the public reader stays where it was.
        write(dir.path(), "Private/B.md", "# B changed\n");
        settled(&built, |i| i.entries["Private/B.md"].title() == "B changed");
        let quiet = since_for(&built, &public, 2, mine.epoch, mine.access, mine.revision).unwrap();
        assert_eq!(quiet.revision, mine.revision, "no count of hidden edits");
        assert!(IndexDelta::from_bytes(&quiet.bytes).unwrap().is_empty());
        let full = since_for(&built, &everything, 1, all.epoch, all.access, all.revision).unwrap();
        assert_eq!(IndexDelta::from_bytes(&full.bytes).unwrap().put.len(), 1);
        // A new note it may read reaches it as a delta, not a snapshot.
        write(dir.path(), "Public/C.md", "# C\n");
        settled(&built, |i| i.entries.contains_key("Public/C.md"));
        let more = since_for(&built, &public, 2, mine.epoch, mine.access, mine.revision).unwrap();
        let more = IndexDelta::from_bytes(&more.bytes).unwrap();
        assert_eq!(
            more.put.iter().map(|e| &*e.path).collect::<Vec<_>>(),
            ["Public/C.md"]
        );

        // What the public reader may read changes: it takes a snapshot.
        assert!(since_for(&built, &public, 3, mine.epoch, mine.access, mine.revision).is_none());
        // Another history (the node started again): a snapshot.
        let again = Built::new(DiskSource::open(dir.path()));
        assert!(since_for(&again, &public, 2, mine.epoch, mine.access, mine.revision).is_none());
    }
}
