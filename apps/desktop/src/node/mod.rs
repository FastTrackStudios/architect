//! The app's node: what makes a running Architect app a device on the
//! account and a sync peer.
//!
//! The app is the node. Its window starts it in the same process
//! (`architect_desktop::run`, `start_node`), so installing the app is all
//! a machine needs. A server runs the same app without the window,
//! `architect-desktop serve` ([`main`]), the one that is always online.
//! Both start it through [`start_parts`] from one [`Config`]; where they
//! differ (where HTTP listens, where state lives, the device's platform,
//! whose session) it is the configuration's defaults
//! ([`config::app_defaults`]), never a second code path.
//!
//! The desktop and mobile apps are peers; the account server only
//! introduces them (accounts, orgs, the device address book, pairing:
//! `features/devices`). No node is a central server: every device syncs
//! with the others the way it syncs with any peer, and they keep working
//! with any one of them stopped.
//!
//! [`start`] runs it:
//!
//! - **A device on the account.** With an issuer and a session
//!   ([`Config::issuer`], [`Config::session_file`]; in the window, else
//!   the app's signed-in account, an [`AccountSource`]) it registers
//!   itself in the account's address book as a device of its
//!   [`Config::platform`] (`server` for `serve`) at its iroh endpoint, offers its org ([`Config::org`]) while the account shares
//!   none, and lets the account's other devices in
//!   (`architect_devices::account::pass`). Again every few minutes, and
//!   every sweep until somebody else is on the account.
//! - **Its lanes over iroh.** The node's documents (`crdt::p2p::node_router`:
//!   `DocSync` for live editing, `DocPeer` for sweeps), to the devices its
//!   `Admission` lets in, each connection closed when its device is
//!   revoked (`architect_devices::serve`).
//! - **A sync peer for the vaults.** Each vault's notes are documents
//!   (`vault_collab::VaultCollab`: a file is a Loro document, written
//!   behind to the file, an outside edit merged in). [`catalog`] names
//!   them on the peer lane. With each peer it dials it keeps a live
//!   `DocSync` bridge per note ([`live`], `crdt::peer::bridge`), so an
//!   edit reaches the others within a round trip; every
//!   [`Config::sweep_secs`] it also pulls from and pushes to each peer
//!   (`crdt::peer::pull_with`), the fallback for whatever a bridge missed.
//!   A note seeded on two nodes separately is settled: the second seed
//!   adopts the first's lineage (`crdt::peer::adopt_lineage`). A vault
//!   file of a document kind its extensions declare ([`kinds`]: a Keyflow
//!   `.session`) is a document of the same registry, kept in the file
//!   itself, and travels the same lanes.
//! - **In an issuer org** ([`org`]). With [`Config::host_org`] it serves
//!   the org's enrolment lane (`/org/{slug}/vox`): a member enrols a
//!   device with a role, lists and revokes; an enrolled device syncs the
//!   vaults as its role allows. With [`Config::enrol_at`] it enrols itself
//!   at another node's lane, once, and syncs with that node.
//! - **Over HTTP** ([`Config::bind`]): `/health` and `/version.json`; the
//!   vaults in [`Config::sites`] as Sites (#364, rebuilt when the vault
//!   changes); [`service::Node`] over same-origin vox (`/vox`) and its
//!   HTTP face (`POST /node/status`); and `/mcp`, with its bearer only,
//!   serving `NODE_STATUS` and the `NOTES_*` actions.
//! - **Share links** ([`shares`], #374): `/s/<code>/` serves a share's
//!   scope of a vault as a Site to the audience it names, until it
//!   expires or is revoked; the org lane serves `ShareLinks` to make,
//!   change and revoke them.

pub mod catalog;
pub mod cli;
pub mod config;
mod files;
mod guest_live;

/// How often a share guest's live session asks again whether its link
/// still serves the document (`/s/<code>/_guest/live`, #847).
#[must_use]
pub const fn guest_live_recheck() -> std::time::Duration {
    guest_live::RECHECK
}
pub mod kinds;
mod live;
pub mod org;
mod s3;
pub mod service;
pub mod share_layers;
pub mod share_shell;
pub mod shares;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use architect::Services as _;
use architect::action::ActionList;
use architect::host::EngineHost;
use architect::iroh_link::{self, PeerLookup, VOX_ALPN};
use architect::layer::Layer as _;
use architect_devices::Admission;
use architect_devices::account::{self, AccountNode, ThisDevice};
use architect_permissions::Principal;
use auth_proto::{AccountOrg, PeerDevice};
use crdt::p2p::{NodeAccess, Shares, node_router_with};
use crdt::peer::{DocAt, DocPeerClient, Foreign, SelfLane};
use tokio::task::JoinHandle;
use vault_live::Backend;

pub use catalog::VaultCatalog;
pub use config::{Config, ConfigError, IrohMode, Placement, Size};
pub use service::{FilesStore, NodeError, NodeState, NodeStatus};

/// The principal this node's own sweeps attach to its registry as.
const SELF: &str = "doc-peer";
/// How long one document's exchange may take in a sweep.
const PER_DOC: Duration = Duration::from_secs(20);
/// How long one peer's whole sweep may take.
const PER_PEER: Duration = Duration::from_secs(120);
/// How often the account pass runs once the account's devices are known.
const ACCOUNT_EVERY: Duration = Duration::from_secs(300);
/// How often the account pass runs while the node is alone on it.
const ACCOUNT_ALONE: Duration = Duration::from_secs(2);
/// How long the vault is left to settle before a Site is rebuilt.
const SITE_SETTLE: Duration = Duration::from_millis(750);

/// Why the node did not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error("the data dir {path}: {source}")]
    Data {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("iroh: {0}")]
    Iroh(String),
    #[error("binding {bind}: {source}")]
    Bind {
        bind: String,
        source: std::io::Error,
    },
    #[error("{0}")]
    Other(String),
}

/// A running node. Dropping it stops it.
pub struct Running {
    /// Where its HTTP side listens.
    pub http: SocketAddr,
    /// Where its S3 face listens, with [`Config::s3_bind`].
    pub s3: Option<SocketAddr>,
    /// Its iroh endpoint.
    pub endpoint: iroh::Endpoint,
    /// What its services answer from.
    pub state: NodeState,
    /// Its files engine, with [`Config::files_dir`].
    pub files: Option<architect_files::sync::Engine>,
    /// The drive, with [`Config::drive`]: unmounted when the node stops.
    #[cfg(unix)]
    pub drive: Option<architect_files::drive::Mounted>,
    tasks: Vec<JoinHandle<()>>,
    registry: crdt::DocRegistry,
    /// Its vaults' documents, for [`Running::kind_documents`].
    collab: vault_collab::VaultCollab,
    /// Its vaults' ids.
    vaults: Vec<String>,
    _watching: Option<vault_live::sync::WatcherHandle>,
    _self_lane: Arc<SelfLane>,
}

/// How a node's extensions reach the presence of a document another node
/// hosts (session#192): dialled over this node's iroh endpoint by the
/// host's endpoint id alone, as any of the org's devices dials another.
#[derive(Clone)]
pub struct NodePresenceLink(iroh::Endpoint);

impl architect_extensions::live::PresenceLink for NodePresenceLink {
    fn me(&self) -> String {
        self.0.id().to_string()
    }

    fn dial(
        &self,
        host: String,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<crdt::sync::DocPresenceClient, String>> + Send>>
    {
        let endpoint = self.0.clone();
        Box::pin(async move {
            let id: iroh::EndpointId = host.parse().map_err(|e| format!("{host}: {e}"))?;
            let link = iroh_link::connect(&endpoint, id)
                .await
                .map_err(|e| e.to_string())?;
            vox::initiator_on(link)
                .establish::<crdt::sync::DocPresenceClient>()
                .await
                .map_err(|e| format!("{e:?}"))
        })
    }
}

impl Running {
    /// How its extensions reach a presence another node hosts
    /// ([`NodePresenceLink`]).
    #[must_use]
    pub fn presence_link(&self) -> NodePresenceLink {
        NodePresenceLink(self.endpoint.clone())
    }

    /// Its endpoint id, as the account's address book keys it.
    #[must_use]
    pub fn endpoint_id(&self) -> String {
        architect_devices::endpoint_key(&self.endpoint.id().to_string())
    }

    /// `endpoint@ip:port` of its first bound socket: a [`Config::peers`]
    /// entry for a node that dials it with no lookup.
    #[must_use]
    pub fn dial(&self) -> Option<String> {
        self.endpoint
            .bound_sockets()
            .into_iter()
            .next()
            .map(|sock| format!("{}@{sock}", self.endpoint.id()))
    }

    /// The document `doc_id` as this node holds it now (a note's, or a
    /// document of an extension's kind: the file's id), opened if it was
    /// not; `None` when this node does not keep it.
    pub fn document(
        &self,
        doc_id: uuid::Uuid,
    ) -> impl Future<Output = Option<crdt::CrdtDoc>> + Send + 'static {
        let registry = self.registry.clone();
        async move { registry.doc(doc_id).await.ok() }
    }

    /// Its vaults' documents of a kind, as the window's extensions reach
    /// them (`Host::set_kind_documents`, `architect_extensions::live`): a
    /// vault path ([`vault_path`]) names the document its file is, and an
    /// extension's session attaches to the vaults' registry as the
    /// extension, under the vault's write guard (not the node's lanes'
    /// device check: the window is this device).
    #[must_use]
    pub fn kind_documents(&self) -> architect_extensions::live::RegistryDocuments {
        let (collab, vaults) = (self.collab.clone(), self.vaults.clone());
        architect_extensions::live::RegistryDocuments::new(
            self.collab.registry().clone(),
            move |path| {
                let (vault, path) = vault_path(&vaults, path)?;
                let doc_id = collab.document_of(vault, path)?;
                Some(architect_extensions::live::KindDoc {
                    doc_id,
                    kind: collab.kind_of(path)?.id,
                })
            },
        )
    }

    /// The body of note `path` in vault `vault`, as its live document
    /// holds it now (the file follows after the write-behind).
    pub fn note_text(
        &self,
        vault: &str,
        path: &str,
    ) -> impl Future<Output = Option<String>> + Send + 'static {
        let registry = self.registry.clone();
        let doc_id = vault_proto::collab_doc_id(vault, path);
        async move {
            let doc = registry.doc(doc_id).await.ok()?;
            Some(
                doc.loro()
                    .get_text(vault_proto::COLLAB_BODY_CONTAINER)
                    .to_string(),
            )
        }
    }

    /// Whether note `path` in vault `vault` is deleted (its document's
    /// tombstone), or `None` while its document is not open: this never
    /// opens it, so it can watch a delete without racing one.
    pub fn note_deleted(
        &self,
        vault: &str,
        path: &str,
    ) -> impl Future<Output = Option<bool>> + Send + 'static {
        use crdt::loro::{LoroValue, ValueOrContainer};
        let registry = self.registry.clone();
        let doc_id = vault_proto::collab_doc_id(vault, path);
        async move {
            if !registry.is_open(doc_id).await {
                return None;
            }
            let doc = registry.doc(doc_id).await.ok()?;
            Some(matches!(
                doc.loro()
                    .get_map(vault_proto::COLLAB_META_CONTAINER)
                    .get(vault_proto::COLLAB_META_DELETED),
                Some(ValueOrContainer::Value(LoroValue::Bool(true)))
            ))
        }
    }

    /// Stop it: its tasks, then its endpoint.
    pub async fn stop(mut self) {
        // The mount's threads call into this runtime: unmount off it.
        #[cfg(unix)]
        if let Some(drive) = self.drive.take() {
            let _ = tokio::task::spawn_blocking(move || drive.unmount()).await;
        }
        for task in self.tasks.drain(..) {
            task.abort();
        }
        self.endpoint.close().await;
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Start the node `config` describes, drawing the blocks of the
/// extensions installed in [`Config::extensions_dir`] and no others.
///
/// Inside a multi-thread tokio runtime.
///
/// # Errors
///
/// [`StartError`]: the data dir, the iroh endpoint or the HTTP listener.
pub async fn start(config: Config) -> Result<Running, StartError> {
    // Boxed: its callers' futures stay the size they were.
    Box::pin(start_with(config, architect_sites::Blocks::new())).await
}

/// Start the node `config` describes, drawing the fences `blocks` claim.
///
/// Its share links and Sites draw them (#488): first those the extensions
/// installed in [`Config::extensions_dir`] claim (an embed page, a WASI
/// component's embed view; `architect_sites::InstalledBlocks`, read again
/// when the folder changes), then `blocks`: a distribution's compiled-in
/// extensions' (`Distribution::blocks`), or an embedder's own
/// (`architect_sites::FnBlock`). Inside a multi-thread tokio runtime.
///
/// # Errors
///
/// [`StartError`]: the data dir, the iroh endpoint or the HTTP listener.
pub async fn start_with(
    config: Config,
    given: architect_sites::Blocks,
) -> Result<Running, StartError> {
    Box::pin(start_with_layers(config, given, Vec::new())).await
}

/// [`start_with`], its share links' files placed in layers (#528).
///
/// By the `[[share_layers]]` rules of the extensions installed in
/// [`Config::extensions_dir`], then by `layers`: a distribution's
/// compiled-in extensions' (`Distribution::share_layers`).
///
/// # Errors
///
/// [`StartError`]: the data dir, the iroh endpoint or the HTTP listener.
pub async fn start_with_layers(
    config: Config,
    given: architect_sites::Blocks,
    layers: Vec<architect_sites::ShareLayersDecl>,
) -> Result<Running, StartError> {
    Box::pin(start_with_islands(config, given, layers, Vec::new())).await
}

/// [`start_with_layers`], with `islands` drawn on its share links'
/// now-playing views (studio#445): a distribution's linked-in WASI
/// components (`Distribution::island`), served at `/_ext/`.
///
/// # Errors
///
/// [`StartError`]: the data dir, the iroh endpoint or the HTTP listener.
pub async fn start_with_islands(
    config: Config,
    given: architect_sites::Blocks,
    layers: Vec<architect_sites::ShareLayersDecl>,
    islands: Vec<architect_sites::SiteIsland>,
) -> Result<Running, StartError> {
    Box::pin(start_with_shell(config, given, layers, islands, Vec::new())).await
}

/// [`start_with_islands`], with note types' `favourites`.
///
/// A distribution's compiled-in extensions' `[[favourites]]`: a share
/// page's phone shell shows the tabs its root note's type declares
/// (`architect_sites::shell`).
///
/// # Errors
///
/// [`StartError`]: the data dir, the iroh endpoint or the HTTP listener.
pub async fn start_with_shell(
    config: Config,
    given: architect_sites::Blocks,
    layers: Vec<architect_sites::ShareLayersDecl>,
    islands: Vec<architect_sites::SiteIsland>,
    favourites: Vec<architect_favourites::Listed>,
) -> Result<Running, StartError> {
    let parts = Parts {
        blocks: given,
        layers,
        islands,
        favourites,
        ..Parts::default()
    };
    Box::pin(start_parts(config, parts)).await
}

/// What a node is given besides its [`Config`]: what its distribution
/// draws and places ([`Parts::of`]), and who it is on the account when
/// the configuration names no session file (the window's signed-in
/// account).
#[derive(Default)]
pub struct Parts {
    /// The fences its compiled-in extensions claim ([`start_with`]).
    pub blocks: architect_sites::Blocks,
    /// Its compiled-in extensions' share layers ([`start_with_layers`]).
    pub layers: Vec<architect_sites::ShareLayersDecl>,
    /// Its linked-in islands ([`start_with_islands`]).
    pub islands: Vec<architect_sites::SiteIsland>,
    /// Note types' favourites ([`start_with_shell`]).
    pub favourites: Vec<architect_favourites::Listed>,
    /// Who it is on the account, when [`Config::issuer`] and
    /// [`Config::session_file`] do not say: those win when set.
    pub account: Option<AccountSource>,
    /// Its compiled-in extensions' document kinds
    /// (`Distribution::doc_kinds`): a vault file of one syncs like a note.
    pub doc_kinds: Vec<architect_extensions::contrib::DocKindDecl>,
    /// The kinds its vaults' documents are of, shared: the node adds its
    /// extensions' ([`kinds`]), its embedder's among them
    /// ([`Self::loaded_kinds`]).
    pub kinds: crdt::SharedDocKinds,
    /// The kinds the embedder's extensions declare while they are loaded
    /// (the window's: its development copies too, [`kinds::follow`]): a
    /// kind it stops declaring goes, unless another source still does.
    pub loaded_kinds: kinds::LoadedKinds,
}

impl Parts {
    /// What `dist`'s compiled-in extensions give a node run on `config`
    /// (their packages under `<data dir>/linked`).
    #[must_use]
    pub fn of(dist: &architect_distribution::Distribution, config: &Config) -> Self {
        Self {
            blocks: dist.blocks(&config.data_dir.join("linked")),
            layers: dist.share_layers(),
            islands: dist.islands(),
            favourites: dist.favourites(),
            account: None,
            doc_kinds: dist.doc_kinds(),
            kinds: crdt::SharedDocKinds::new(),
            loaded_kinds: kinds::LoadedKinds::new(),
        }
    }

    /// The same, on the account as `account`.
    #[must_use]
    pub fn with_account(mut self, account: AccountSource) -> Self {
        self.account = Some(account);
        self
    }
}

/// Who a node is on the account: an issuer's origin and a session token.
///
/// Asked again at each use, so a sign-in, a sign-out or a switch
/// of account is followed with nothing to restart. `None` while nobody is
/// signed in.
#[derive(Clone)]
pub struct AccountSource(Arc<dyn Fn() -> Option<(String, String)> + Send + Sync>);

impl AccountSource {
    /// Asks `now` each time.
    pub fn new(now: impl Fn() -> Option<(String, String)> + Send + Sync + 'static) -> Self {
        Self(Arc::new(now))
    }

    /// [`Config::issuer`] and the token in [`Config::session_file`], when
    /// both are set (a server's, the local stack's).
    #[must_use]
    pub fn of_config(config: &Config) -> Option<Self> {
        let (issuer, file) = (config.issuer.clone()?, config.session_file.clone()?);
        Some(Self::new(move || {
            read_secret(&file).map(|token| (issuer.clone(), token))
        }))
    }

    /// The issuer and the session, now.
    #[must_use]
    pub fn now(&self) -> Option<(String, String)> {
        (self.0)()
    }
}

impl std::fmt::Debug for AccountSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccountSource")
    }
}

/// The node `config` describes, with `parts`: [`start`] and its
/// siblings, the window's node and `serve` all start through here.
///
/// # Errors
///
/// [`StartError`]: the data dir, the iroh endpoint or the HTTP listener.
pub async fn start_parts(config: Config, parts: Parts) -> Result<Running, StartError> {
    let Parts {
        blocks: given,
        layers,
        islands,
        favourites,
        account,
        doc_kinds,
        kinds,
        loaded_kinds,
    } = parts;
    let data = &config.data_dir;
    // An installed extension draws over a compiled-in one claiming the
    // same fence, as its development copy does in the app.
    let mut blocks = architect_sites::Blocks::new();
    blocks.add(Arc::new(architect_sites::InstalledBlocks::new(
        config.extensions_dir(),
    )));
    blocks.add(Arc::new(given));
    let made = |path: &Path| {
        std::fs::create_dir_all(path).map_err(|source| StartError::Data {
            path: path.to_owned(),
            source,
        })
    };
    made(data)?;

    // ── The vaults ────────────────────────────────────────────────────
    let mut roots = std::collections::HashMap::new();
    for (id, path) in &config.vaults {
        let root = config.vault_root(path);
        made(&root)?;
        roots.insert(id.clone(), root);
    }
    let backend = Backend::with_roots(roots.clone());
    // ── Projects and asset libraries: manifests in the same registry ──
    let files_registry: Arc<std::sync::OnceLock<crdt::DocRegistry>> = Arc::default();
    let files = config
        .files_dir
        .as_ref()
        .map(|dir| files::engine(&config, dir, files_registry.clone()))
        .transpose()
        .map_err(StartError::Other)?;
    let collab =
        vault_collab::VaultCollab::with_kinds(backend.clone(), data.join("crdt"), kinds.clone());
    let extension_kinds = kinds::ExtensionKinds::new(
        kinds,
        doc_kinds,
        Some(config.extensions_dir()),
        loaded_kinds,
    );
    let collab = match &files {
        Some(engine) => {
            let engine = engine.clone();
            collab.with_plain_docs(move |doc_id| engine.knows(doc_id))
        }
        None => collab,
    };
    for id in config.vaults.keys() {
        collab.watch_vault(id);
    }
    let ids: Vec<String> = config.vaults.keys().cloned().collect();
    let watching = if ids.is_empty() {
        None
    } else {
        match backend.start_watchers(&ids).await {
            Ok(handle) => Some(handle),
            Err(error) => {
                tracing::warn!(%error, "the vaults are not watched: edits on disk wait for a restart");
                None
            }
        }
    };

    // ── Who it lets in, and its lanes over iroh ───────────────────────
    let admission = Admission::new();
    keep_devices(data, &admission);
    // The configuration's session first; else the one the node is given.
    let account = AccountSource::of_config(&config).or(account);
    let signed = account.as_ref().and_then(AccountSource::now);
    let issuer = config
        .issuer
        .clone()
        .or_else(|| signed.as_ref().map(|(issuer, _)| issuer.clone()));
    let node_org = match (config.vault_org(), &signed) {
        (Some(slug), Some((issuer, token))) => node_org(data, issuer, token, slug).await,
        (Some(slug), _) => org::remembered(data, slug),
        _ => None,
    };
    let lane = match (&node_org, &issuer) {
        (Some(org), Some(issuer)) => Some(
            org::OrgLane::new(org.clone(), &admission, issuer)
                .with_recheck(Duration::from_secs(config.lane_recheck_secs.max(1)))
                .with_issuer_grace(Duration::from_secs(config.lane_issuer_grace_secs))
                .with_origins(config.lane_origins.clone()),
        ),
        _ => None,
    };
    let mut access = NodeAccess::new(admission.clone(), Shares::new());
    if let Some(lane) = &lane {
        let engine: Arc<dyn architect_permissions::PermissionEngine> = lane.engine.clone();
        access = access.org_docs(lane.org.id.0.clone(), engine);
    }
    // Who may open a document: the node's links and devices, itself, the
    // org's members; and a vault share link's guest, read-only, for a
    // document its link serves while its session lasts (#847).
    let opens: Arc<dyn Fn(Option<&Principal>, uuid::Uuid) -> bool + Send + Sync> = {
        let (access, lane) = (access.clone(), lane.clone());
        Arc::new(move |who, doc_id| {
            access.admits(who, doc_id)
                || matches!(who, Some(Principal::Service { name }) if name == SELF)
                || matches!((who, &lane), (Some(who), Some(lane)) if lane.opens(who, doc_id))
        })
    };
    let guest_grants = guest_live::GuestGrants::default();
    let registry = collab
        .registry()
        .clone()
        .with_authorizer({
            let (opens, grants) = (opens.clone(), guest_grants.clone());
            move |who, doc_id| opens(who, doc_id) || grants.holds(who, doc_id)
        })
        // A share guest never writes: only the rule above without it.
        .with_write_authorizer(move |who, doc_id| opens(who, doc_id));
    let _ = files_registry.set(registry.clone());
    let catalog = VaultCatalog::new(backend.clone())
        .with_files(files.clone())
        .with_documents(collab.clone(), extension_kinds);
    let endpoint = bind_iroh(&config).await?;
    let endpoint_id = architect_devices::endpoint_key(&endpoint.id().to_string());
    let mut tasks = Vec::new();
    tasks.push(tokio::spawn({
        let (endpoint, registry, catalog, access, admission, files) = (
            endpoint.clone(),
            registry.clone(),
            catalog.clone(),
            access.clone(),
            admission.clone(),
            files.clone(),
        );
        async move {
            architect_devices::serve(&endpoint, admission, move |remote| {
                node_router_with(
                    registry.clone(),
                    catalog.clone(),
                    access.clone(),
                    &remote.to_string(),
                    files.as_ref().map(files::more).unwrap_or_default(),
                )
            })
            .await;
        }
    }));

    // The share links' layer rules (#528): the shares place files by them,
    // the status says what the owner should know of them.
    let share_layers = share_layers::LayerRules::new(
        Some(config.extensions_dir()),
        layers,
        Some(data.join("share-layers.json")),
    );
    let state = NodeState {
        layers: Some(share_layers.clone()),
        name: config.name.clone(),
        endpoint: endpoint_id.clone(),
        iroh: Some(endpoint.clone()),
        backend: backend.clone(),
        sites: config.sites.keys().cloned().collect(),
        admission: admission.clone(),
        account: Arc::new(Mutex::new(service::AccountView::default())),
        files: files.clone(),
        hosts: config.host_org.clone(),
        indexed: service::Indexed::default(),
    };

    // ── The account ──────────────────────────────────────────────────
    let every = Duration::from_secs(config.sweep_secs.max(1));
    if let Some(account) = &account {
        let me = ThisDevice {
            device_id: device_id(data)?,
            name: config.name.clone(),
            platform: config.platform.clone(),
            relay_url: None,
            retire: None,
        };
        let node = OnAccount {
            endpoint: endpoint_id.clone(),
            offer: config.org.as_deref().map(offered_org),
        };
        tasks.push(tokio::spawn(keep_on_account(
            account.clone(),
            me,
            node,
            state.clone(),
            every,
        )));
    } else {
        tracing::info!("no issuer or session: the node is on no account, and lets nobody in");
    }

    // ── Enrolling itself in an org another node hosts ────────────────
    if let (Some(at), Some(lane), Some(account)) = (&config.enrol_at, &lane, &account) {
        let marker = org::enrolled_marker(data);
        if marker.exists() {
            tracing::debug!("enrolled before: not enrolling again");
        } else {
            let enrol = org::EnrolAt {
                at: at.clone(),
                org: lane.org.clone(),
                role: architect_orgs::Role::parse(&config.enrol_role),
                endpoint: endpoint_id.clone(),
                name: config.name.clone(),
                platform: config.platform.clone(),
            };
            tasks.push(tokio::spawn(enrol_until_done(
                enrol,
                account.clone(),
                marker,
                admission.clone(),
                every,
            )));
        }
    }

    // ── Sweeps ───────────────────────────────────────────────────────
    let self_lane = Arc::new(
        SelfLane::open(
            registry.clone(),
            Principal::Service {
                name: SELF.to_owned(),
            },
        )
        .await
        .map_err(StartError::Other)?,
    );
    let dials = parse_peers(&config.peers);
    if let Some(engine) = &files {
        let every = Duration::from_secs(config.files_secs.max(1));
        let bucket = files::backend(&config).await.map_err(StartError::Other)?;
        let mut fixed: Vec<Arc<dyn architect::blob::Remote>> = Vec::new();
        if let Some(s3) = bucket {
            // The bucket is a holder in the ledger, attested as it changes.
            let holder = architect_files::sync::BucketHolder::new(
                engine.clone(),
                &s3,
                config.blob_s3_endpoint.as_deref(),
            );
            let s3 = s3.with_events(Arc::new(holder));
            fixed.push(Arc::new(s3.clone()));
            tasks.push(tokio::spawn(files::replicate_forever(
                engine.clone(),
                s3,
                config.blob_placement,
                config.blob_cache_mb.saturating_mul(1 << 20),
                every,
            )));
        }
        tasks.push(tokio::spawn(engine.clone().run(every)));
        tasks.push(tokio::spawn(files::dial_forever(
            engine.clone(),
            endpoint.clone(),
            dials.clone(),
            admission.clone(),
            config.iroh,
            fixed,
        )));
    }
    tasks.push(tokio::spawn(live::live_forever(live::Live {
        endpoint: endpoint.clone(),
        lane: Arc::clone(&self_lane),
        catalog: catalog.clone(),
        admission: admission.clone(),
        configured: dials.clone(),
        mode: config.iroh,
        every,
        max: config.live_max,
    })));
    tasks.push(tokio::spawn(sweep_forever(
        endpoint.clone(),
        Arc::clone(&self_lane),
        catalog.clone(),
        admission.clone(),
        dials,
        config.iroh,
        every,
    )));

    // ── Share links ──────────────────────────────────────────────────
    let shares = shares::Shares::new(
        backend.clone(),
        data,
        node_org.clone(),
        issuer.as_deref(),
        blocks.clone(),
        share_layers,
    )
    .with_islands(islands)
    .with_favourites(favourites)
    .with_web_app({
        let (web, why) = share_shell::WebApp::from_setting(config.web_app.as_deref());
        if let Some(why) = why {
            tracing::warn!("{why}");
        }
        web
    });
    for id in config.vaults.keys() {
        tasks.push(tokio::spawn(shares.clone().keep_built(id.clone())));
    }
    // Only a share's owner writes what its link runs (#710): its
    // extensions file and packages, whoever else may write the vault.
    {
        let shares = shares.clone();
        let _ = backend.set_write_guard(Arc::new(vault_live::ExtensionPaths::new(
            move |vault, code| {
                shares
                    .list()
                    .into_iter()
                    .find(|s| {
                        s.vault == vault
                            && (s.code == code || s.audiences.iter().any(|a| a.code == code))
                    })
                    .map(|s| s.created_by)
            },
        )));
    }

    // ── Sites ────────────────────────────────────────────────────────
    let mut sites = Vec::new();
    for (mount, vault) in &config.sites {
        let Some(root) = roots.get(vault).cloned() else {
            tracing::warn!(%mount, %vault, "a Site of a vault this node does not hold: not served");
            continue;
        };
        let out = data.join("sites").join(vault);
        let base = format!("{}/", mount.trim_end_matches('/'));
        tasks.push(tokio::spawn(keep_site_built(
            backend.clone(),
            vault.clone(),
            root,
            out.clone(),
            base,
            blocks.clone(),
        )));
        sites.push((mount.clone(), out));
    }

    // ── HTTP ─────────────────────────────────────────────────────────
    // Ungated: the router holds `Node` alone, whose one method says what
    // any device that can reach the node may know (its name, endpoint,
    // counts). The vaults travel on the iroh lanes, which admit only the
    // account's devices, and over `/mcp`, behind its bearer.
    let mut host = EngineHost::ungated(state.clone().into_router(), config.bind.clone())
        .build_info(architect::build_info!())
        .plugin(state.clone().into_http_router());
    for (mount, out) in &sites {
        host = host.site(mount, out);
    }
    if let Some(engine) = &files {
        host = files::mount(host, engine, &config.bind);
    }
    let mut s3_addr = None;
    if let (Some(engine), Some(bind)) = (&files, &config.s3_bind) {
        let (addr, task) = s3::serve(engine.clone(), data, admission.clone(), bind)
            .await
            .map_err(|source| StartError::Bind {
                bind: bind.clone(),
                source,
            })?;
        tracing::info!(%addr, "files over S3");
        s3_addr = Some(addr);
        tasks.push(task);
    }
    if let (Some(lane), Some(_)) = (&lane, &config.host_org) {
        // The notes, and their index for a browser's queries (it runs them
        // itself, over its copy).
        // Each note in the index only for whom the lane's rules let read it.
        let (reads, key) = (lane.clone(), lane.clone());
        state.indexed.set_access(service::Access {
            reads: Arc::new(move |who, vault, path| reads.reads(who, vault, path)),
            key: Arc::new(move |who| key.read_key(who)),
        });
        let notes = architect::layers![
            service::notes::NotesService,
            service::index_sync::VaultIndexSyncService
        ]
        .provide(state.clone());
        host = lane
            .clone()
            .with_vaults(registry.clone(), catalog.clone(), notes)
            .with_shares(shares.clone())
            .with_node_actions(node_actions(&config))
            .mount(host);
    }
    // A share link's guests (#629): `POST /s/<code>/_guest/<method>`,
    // which the gate passes through, each call checked as the link's
    // pages are.
    host = host.plugin(shares::GuestLane(shares.clone()).into_http_router());
    // Their live documents, read-only (#847): `/s/<code>/_guest/live`.
    host = guest_live::mount(
        host,
        shares::GuestLane(shares.clone()),
        registry.clone(),
        collab.clone(),
        guest_grants,
    );
    // Universal links: the iOS apps this node's links open in, from its
    // configuration; absent when none is configured.
    if let Some(body) = shares::apple_app_site_association(&config.apple_app_ids) {
        host = host.plugin(architect::http::axum::Router::new().route(
            "/.well-known/apple-app-site-association",
            architect::http::axum::routing::get(move || {
                let body = body.clone();
                async move {
                    (
                        [(
                            architect::http::axum::http::header::CONTENT_TYPE,
                            "application/json",
                        )],
                        body,
                    )
                }
            }),
        ));
    }
    if let Some(file) = &config.mcp_token_file {
        match read_secret(file) {
            Some(token) => {
                let list = ActionList::new();
                service::node::node_actions::register(state.clone(), &list)
                    .and_then(|()| service::notes::notes_actions::register(state.clone(), &list))
                    .and_then(|()| {
                        files.as_ref().map_or(Ok(()), |engine| {
                            architect_files::agent::files::files_actions::register(
                                engine.clone(),
                                &list,
                            )
                        })
                    })
                    .map_err(|e| StartError::Other(format!("the node's actions: {e:?}")))?;
                let server =
                    architect_mcp::Server::new("architect-node", env!("CARGO_PKG_VERSION"), list);
                host = host.plugin(architect_mcp::http::router(server, Some(token)));
            }
            None => {
                tracing::warn!(file = %file.display(), "no MCP bearer in the file: /mcp is not served");
            }
        }
    }
    // Outermost: a share's link and domain are answered before anything
    // else is matched (a Site mounted at `/` included).
    // Its Sites' pages carry the shares' Content-Security-Policy (#520).
    let site_mounts: Arc<Vec<String>> = Arc::new(sites.iter().map(|(m, _)| m.clone()).collect());
    let host = host.finish({
        let shares = shares.clone();
        move |app| {
            app.layer(architect::http::axum::middleware::from_fn_with_state(
                site_mounts,
                shares::sites_policy,
            ))
            .layer(architect::http::axum::middleware::from_fn_with_state(
                shares,
                shares::gate,
            ))
        }
    });
    let app = host.into_app();
    let listener = tokio::net::TcpListener::bind(&config.bind)
        .await
        .map_err(|source| StartError::Bind {
            bind: config.bind.clone(),
            source,
        })?;
    let http = listener.local_addr().map_err(|source| StartError::Bind {
        bind: config.bind.clone(),
        source,
    })?;
    tasks.push(tokio::spawn(async move {
        if let Err(error) = architect::http::axum::serve(listener, app).await {
            tracing::error!(%error, "the node's HTTP side stopped");
        }
    }));
    tracing::info!(
        name = %config.name,
        endpoint = %endpoint_id,
        %http,
        vaults = config.vaults.len(),
        sites = sites.len(),
        "the node is up"
    );
    #[cfg(unix)]
    let drive = match (&files, &config.drive) {
        (Some(engine), Some(at)) => files::mount_drive(engine, &config, at),
        _ => None,
    };
    Ok(Running {
        http,
        s3: s3_addr,
        endpoint,
        state,
        files,
        #[cfg(unix)]
        drive,
        tasks,
        registry,
        collab,
        vaults: config.vaults.keys().cloned().collect(),
        _watching: watching,
        _self_lane: self_lane,
    })
}

/// Which of `vaults` the extension's path `path` is in, and its path there.
///
/// With one vault, the path is in it (as the window's vault-relative paths
/// are); with several, its first folder names the vault
/// (`notes/Songs/a.session`).
#[must_use]
pub fn vault_path<'a>(vaults: &'a [String], path: &'a str) -> Option<(&'a str, &'a str)> {
    if let [only] = vaults {
        return Some((only.as_str(), path));
    }
    let (vault, rest) = path.split_once('/')?;
    vaults
        .iter()
        .find(|v| *v == vault)
        .map(|v| (v.as_str(), rest))
}

/// `architect-desktop serve [--config <file>]`: the node of `dist` until
/// it is stopped.
///
/// Its share links and Sites draw what its compiled-in extensions claim
/// (`Distribution::blocks`, their packages under `<data dir>/linked`).
#[must_use]
pub fn main(
    args: impl Iterator<Item = OsString>,
    dist: &architect_distribution::Distribution,
) -> ExitCode {
    let mut file = std::env::var_os("ARCHITECT_CONFIG").map(PathBuf::from);
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--config") => file = args.next().map(PathBuf::from),
            Some("s3-key") => {
                let rest: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
                return match Config::load(file.as_deref(), |var| std::env::var(var).ok()) {
                    Ok(config) => s3::command(&config, &rest),
                    Err(error) => {
                        let _ = writeln!(std::io::stderr(), "architect-desktop serve: {error}");
                        ExitCode::from(78)
                    }
                };
            }
            Some("--help" | "-h") => {
                let _ = writeln!(
                    std::io::stdout(),
                    "architect-desktop serve [--config <file>] [s3-key <issue|list|revoke> …]\n\n\
                     The Architect app's node with no window (a server).\n\
                     Configured by the file and ARCHITECT_* variables; see\n\
                     docs/content/guide/deploy.md."
                );
                return ExitCode::SUCCESS;
            }
            _ => {
                let _ = writeln!(
                    std::io::stderr(),
                    "architect-desktop serve: unknown argument {}",
                    arg.to_string_lossy()
                );
                return ExitCode::from(64);
            }
        }
    }
    architect::host::boot("info");
    let config = match Config::load(file.as_deref(), |var| std::env::var(var).ok()) {
        Ok(config) => config,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "architect-desktop serve: {error}");
            return ExitCode::from(78);
        }
    };
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = writeln!(std::io::stderr(), "architect-desktop serve: {error}");
            return ExitCode::from(70);
        }
    };
    // Its linked extensions' note file types, before any vault is read.
    for t in dist.file_types() {
        t.register();
    }
    let parts = Parts::of(dist, &config);
    runtime.block_on(async move {
        match start_parts(config, parts).await {
            Ok(running) => {
                let _ = writeln!(
                    std::io::stdout(),
                    "architect node {} on http://{}",
                    running.endpoint_id(),
                    running.http
                );
                stopped().await;
                running.stop().await;
                ExitCode::SUCCESS
            }
            Err(error) => {
                let _ = writeln!(std::io::stderr(), "architect-desktop serve: {error}");
                ExitCode::from(69)
            }
        }
    })
}

/// Until SIGTERM (a container's stop: as PID 1 nothing ends the node
/// for it) or Ctrl-C.
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = term.recv() => {}
                _ = tokio::signal::ctrl_c() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

async fn bind_iroh(config: &Config) -> Result<iroh::Endpoint, StartError> {
    let key = iroh_link::load_or_create_secret_key(&config.data_dir.join("iroh.key"))
        .map_err(|e| StartError::Iroh(format!("its key: {e}")))?;
    let builder = match config.iroh {
        IrohMode::N0 => iroh_link::endpoint_builder(PeerLookup::N0),
        // No relay, no lookup, and none of the builder's default sockets
        // on every interface (`0.0.0.0`, `[::]`): loopback alone, below.
        IrohMode::Local => iroh::Endpoint::builder(iroh::endpoint::presets::Minimal)
            .relay_mode(iroh::RelayMode::Disabled)
            .clear_ip_transports()
            .alpns(vec![VOX_ALPN.to_vec()]),
    }
    .secret_key(key);
    let bind = config
        .iroh_bind
        .clone()
        .or_else(|| (config.iroh == IrohMode::Local).then(|| "127.0.0.1:0".to_owned()));
    let builder = match bind {
        Some(addr) => builder
            .bind_addr(addr.as_str())
            .map_err(|e| StartError::Iroh(format!("{addr}: {e}")))?,
        None => builder,
    };
    builder
        .bind()
        .await
        .map_err(|e| StartError::Iroh(e.to_string()))
}

/// The node's org: asked of the issuer and kept, else as kept.
async fn node_org(data: &Path, issuer: &str, token: &str, slug: &str) -> Option<org::NodeOrg> {
    match tokio::time::timeout(Duration::from_secs(15), org::resolve(issuer, token, slug)).await {
        Ok(Ok(found)) => {
            org::remember(data, &found);
            Some(found)
        }
        Ok(Err(why)) => {
            tracing::warn!(%slug, %why, "the org is not the session's: no org lane");
            org::remembered(data, slug)
        }
        Err(_) => {
            tracing::warn!(%slug, "the issuer did not answer for the org: using the one kept");
            org::remembered(data, slug)
        }
    }
}

/// The devices the node lets in, from `devices.json` and kept there
/// after every change, so it starts offline with the devices it knew
/// (an org's enrolments above all: no issuer holds them).
fn keep_devices(data: &Path, admission: &Admission) {
    let file = data.join("devices.json");
    if let Ok(json) = std::fs::read_to_string(&file)
        && let Err(why) = admission.restore_json(&json)
    {
        tracing::warn!(%why, "devices.json is not a device table: starting with none");
    }
    let mut changes = admission.changes();
    let admission = admission.clone();
    tokio::spawn(async move {
        while changes.changed().await.is_ok() {
            match admission.to_json() {
                Ok(json) => {
                    if let Err(error) = std::fs::write(&file, json) {
                        tracing::warn!(%error, "devices.json is not written");
                    }
                }
                Err(why) => tracing::warn!(%why, "the device table did not encode"),
            }
        }
    });
}

/// Enrol at the hosting node until it answers, then leave the marker.
async fn enrol_until_done(
    enrol: org::EnrolAt,
    account: AccountSource,
    marker: PathBuf,
    admission: Admission,
    retry: Duration,
) {
    loop {
        let outcome = match account.now() {
            Some((_, token)) => org::enrol_once(&enrol, &token, &admission).await,
            None => Err("no session".to_owned()),
        };
        match outcome {
            Ok(peer) => {
                tracing::info!(at = %enrol.at, org = %enrol.org.slug, endpoint = %peer.endpoint, "enrolled in the org");
                if let Err(error) = std::fs::write(&marker, &enrol.at) {
                    tracing::warn!(%error, "the enrolment marker is not written");
                }
                return;
            }
            Err(why) => tracing::warn!(at = %enrol.at, %why, "enrolling in the org; trying again"),
        }
        tokio::time::sleep(retry).await;
    }
}

/// The install's id on the account, kept in the data dir.
fn device_id(data: &Path) -> Result<uuid::Uuid, StartError> {
    let path = data.join("device-id");
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(id) = uuid::Uuid::parse_str(text.trim())
    {
        return Ok(id);
    }
    let id = uuid::Uuid::new_v4();
    std::fs::write(&path, id.to_string()).map_err(|source| StartError::Data { path, source })?;
    Ok(id)
}

/// A secret mounted at `path`, trimmed; never logged.
fn read_secret(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// The issuer's vox lane: `https://x` → `wss://x/vox`.
fn vox_url(issuer: &str) -> String {
    let issuer = issuer.trim().trim_end_matches('/');
    let ws = issuer
        .strip_prefix("https://")
        .map(|rest| format!("wss://{rest}"))
        .or_else(|| {
            issuer
                .strip_prefix("http://")
                .map(|rest| format!("ws://{rest}"))
        })
        .unwrap_or_else(|| issuer.to_owned());
    format!("{ws}/vox")
}

/// The node's side of the account: always at its one endpoint, and
/// offering the org it was configured with while the account shares none.
struct OnAccount {
    endpoint: String,
    offer: Option<AccountOrg>,
}

impl AccountNode for OnAccount {
    fn offer(&self) -> Option<AccountOrg> {
        self.offer.clone()
    }

    async fn hold(
        &self,
        _: Option<&AccountOrg>,
        _: &[PeerDevice],
    ) -> Result<Option<String>, String> {
        Ok(Some(self.endpoint.clone()))
    }
}

/// The org `slug` names, as offered to the account: its id derived from
/// the slug, so every start of the node offers the same one.
fn offered_org(slug: &str) -> AccountOrg {
    AccountOrg {
        id: uuid::Uuid::new_v5(
            &uuid::Uuid::NAMESPACE_URL,
            format!("architect-org:{slug}").as_bytes(),
        ),
        slug: slug.to_owned(),
        display_name: slug.to_owned(),
    }
}

/// The account pass, now and again: this node in the address book, the
/// account's other devices let in.
async fn keep_on_account(
    source: AccountSource,
    me: ThisDevice,
    node: OnAccount,
    state: NodeState,
    retry: Duration,
) {
    let pool = architect::connect::Pool::new();
    loop {
        // Asked each pass: the window's account may sign in, out or change.
        let Some((issuer, token)) = source.now() else {
            architect::lock(&state.account).signed_in = false;
            tracing::debug!("no session: the node is on no account until one signs in");
            tokio::time::sleep(retry).await;
            continue;
        };
        let target = architect::connect::Endpoint::url(vox_url(&issuer));
        let outcome = async {
            let devices: auth_proto::DeviceServiceClient =
                pool.client(&target).await.map_err(|e| e.to_string())?;
            account::pass(&devices, &token, &me, &node, &state.admission)
                .await
                .map_err(|e| e.to_string())
        }
        .await;
        let wait = match outcome {
            Ok(report) => {
                let alone = report.admitted.is_empty();
                {
                    let mut view = architect::lock(&state.account);
                    view.org = report.org.map(|o| o.slug);
                    view.signed_in = true;
                }
                tracing::debug!(admitted = report.admitted.len(), "account pass");
                if alone {
                    retry.min(ACCOUNT_ALONE)
                } else {
                    ACCOUNT_EVERY
                }
            }
            Err(why) => {
                pool.evict(&target);
                architect::lock(&state.account).signed_in = false;
                tracing::warn!(%why, "account pass failed; trying again");
                retry
            }
        };
        tokio::time::sleep(wait).await;
    }
}

/// A peer to dial: its endpoint id, and its address when known.
#[derive(Debug, Clone)]
struct Dial {
    endpoint: String,
    addr: iroh::EndpointAddr,
}

fn parse_peers(peers: &[String]) -> Vec<Dial> {
    peers
        .iter()
        .filter_map(|entry| {
            let (id, sock) = entry.split_once('@')?;
            let id = iroh::EndpointId::from_str(id.trim()).ok()?;
            let sock: SocketAddr = sock.trim().parse().ok()?;
            Some(Dial {
                endpoint: architect_devices::endpoint_key(&id.to_string()),
                addr: iroh::EndpointAddr::from_parts(id, [iroh::TransportAddr::Ip(sock)]),
            })
        })
        .inspect(|d| tracing::debug!(peer = %d.endpoint, "a configured peer"))
        .collect()
}

/// Who to sweep: the configured peers, and (with a lookup) every device
/// the node lets in.
fn targets(configured: &[Dial], admission: &Admission, mode: IrohMode) -> Vec<Dial> {
    let mut out: BTreeMap<String, Dial> = configured
        .iter()
        .map(|d| (d.endpoint.clone(), d.clone()))
        .collect();
    if mode == IrohMode::N0 {
        for peer in admission.list() {
            if out.contains_key(&peer.endpoint) {
                continue;
            }
            if let Ok(id) = iroh::EndpointId::from_str(&peer.endpoint) {
                out.insert(
                    peer.endpoint.clone(),
                    Dial {
                        endpoint: peer.endpoint.clone(),
                        addr: iroh::EndpointAddr::from(id),
                    },
                );
            }
        }
    }
    out.into_values().collect()
}

/// Every `every`, a sweep of each peer it can dial.
async fn sweep_forever(
    endpoint: iroh::Endpoint,
    lane: Arc<SelfLane>,
    catalog: VaultCatalog,
    admission: Admission,
    configured: Vec<Dial>,
    mode: IrohMode,
    every: Duration,
) {
    let me = architect_devices::endpoint_key(&endpoint.id().to_string());
    loop {
        for peer in targets(&configured, &admission, mode) {
            if peer.endpoint == me {
                continue;
            }
            let swept =
                tokio::time::timeout(PER_PEER, sweep(&endpoint, &lane, &catalog, &peer)).await;
            match swept {
                Ok(Ok(report)) if report.exchanged > 0 => tracing::info!(
                    peer = %peer.endpoint,
                    listed = report.listed,
                    exchanged = report.exchanged,
                    pulled = report.pulled,
                    adopted = report.adopted,
                    failed = report.failed,
                    "swept a peer"
                ),
                Ok(Ok(_)) => {}
                Ok(Err(why)) => tracing::debug!(peer = %peer.endpoint, %why, "sweep"),
                Err(_) => tracing::debug!(peer = %peer.endpoint, "sweep timed out"),
            }
        }
        tokio::time::sleep(every).await;
    }
}

/// One sweep of `peer`: register what it holds here, then exchange.
async fn sweep(
    endpoint: &iroh::Endpoint,
    lane: &SelfLane,
    catalog: &VaultCatalog,
    peer: &Dial,
) -> Result<crdt::peer::PullReport, String> {
    let link = iroh_link::connect(endpoint, peer.addr.clone())
        .await
        .map_err(|e| e.to_string())?;
    let client = vox::initiator_on(link)
        .establish::<DocPeerClient>()
        .await
        .map_err(|e| format!("{e:?}"))?;
    let theirs = match client.list().await {
        Ok(theirs) => theirs,
        Err(e) => {
            // Refused (not unreachable): this node was let out, so
            // nothing that peer listed is this node's to keep.
            let why = format!("{e:?}");
            if matches!(e, vox::VoxError::User(_)) || why.contains("permission denied") {
                catalog.listed_by(&peer.endpoint, &[]);
            }
            return Err(why);
        }
    };
    // What this peer lets this node open: a root or ledger it listed
    // before and no longer does is forgotten (access revoked).
    catalog.listed_by(&peer.endpoint, &theirs);
    for held in &theirs {
        let at = DocAt {
            doc_id: held.doc_id,
            name: held.name.clone(),
        };
        if catalog.register(&at).is_err() {
            tracing::debug!(name = %held.name, "a peer's document of a vault this node does not hold");
        }
    }
    crdt::peer::pull_with(
        lane,
        &client,
        &catalog.digests(),
        PER_DOC,
        Foreign::AdoptFirst,
    )
    .await
    .map_err(|e| format!("{e:?}"))
}

/// Build `vault`'s Site into `out` now, and again whenever it changes.
async fn keep_site_built(
    backend: Backend,
    vault: String,
    root: PathBuf,
    out: PathBuf,
    base: String,
    blocks: architect_sites::Blocks,
) {
    let build = move || {
        let mut opts = architect_sites::Options::new(&root, String::new());
        opts.base.clone_from(&base);
        match architect_sites::build(&opts, &blocks, &out) {
            Ok(report) => tracing::debug!(?report, "a Site built"),
            Err(error) => tracing::warn!(%error, "a Site did not build"),
        }
    };
    let mut changes = backend.channel(&vault).await.subscribe();
    let _ = tokio::task::spawn_blocking(build.clone()).await;
    loop {
        match changes.recv().await {
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                tokio::time::sleep(SITE_SETTLE).await;
                while changes.try_recv().is_ok() {}
                let _ = tokio::task::spawn_blocking(build.clone()).await;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Installed extensions' node actions (#583): the node part of each
/// package in [`Config::extensions_dir`], with the node's secrets from its
/// operator's folder ([`Config::extension_secrets`]) or, with none, set by
/// an org admin on the lane and kept in the data dir. Read once at start:
/// a deployment that changes its extensions restarts the node.
fn node_actions(config: &Config) -> architect_extensions::node::NodeHost {
    let node = config.extension_secrets.as_ref().map_or_else(
        || {
            architect_extensions::node::NodeHost::new(Some(
                config.data_dir.join("extension-secrets.json"),
            ))
        },
        architect_extensions::node::NodeHost::with_operator_secrets,
    );
    let dir = config.extensions_dir();
    let (loaded, skipped) = node.load_folder(&dir);
    for (folder, why) in &skipped {
        if !matches!(why, architect_extensions::node::Skipped::NoNodePart) {
            tracing::warn!(folder = %folder.display(), ?why, "an extension's node part is not served");
        }
    }
    tracing::info!(
        extensions = %dir.display(),
        node_actions = ?loaded,
        operator_secrets = config.extension_secrets.is_some(),
        "extensions' node actions"
    );
    node
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_issuers_vox_lane_is_on_its_origin() {
        assert_eq!(vox_url("https://auth.example/"), "wss://auth.example/vox");
        assert_eq!(vox_url("http://127.0.0.1:9"), "ws://127.0.0.1:9/vox");
    }

    #[test]
    fn a_peer_without_an_address_is_not_dialled_offline() {
        let admission = Admission::new();
        assert!(
            targets(
                &parse_peers(&["nonsense".to_owned()]),
                &admission,
                IrohMode::Local
            )
            .is_empty()
        );
    }
}
