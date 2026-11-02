//! The files feature a server mounts (feature `server`, native).
//!
//! [`FilesServer`] keeps every org's files: each org's roots, trees,
//! versions, labels, grants, uploads in flight and quota under
//! `<data>/org/<id>/files/` ([`OrgId::path`]), written atomically after
//! every change; the bytes in a content-addressed [`LocalFs`] blob store,
//! which a server shares with its documents' blobs.
//!
//! A call runs in the org of the lane it arrived on
//! ([`architect_orgs::server::confine`]; a server for one org may name a
//! [`FilesServer::default_org`] instead) and acts for the principal the
//! permissions gate resolved ([`architect::permissions_gate::caller`]).
//! What that person may do is their role in the org — from
//! [`Members`], which the gate's `MembersResolver` fills from the
//! identity server — and their grants ([`crate::access`]). A call with no
//! principal is refused, never treated as the server's own.
//!
//! ```ignore
//! let files = FilesServer::new(data_dir, LocalFs::new(data_dir.join("blobs")), members.clone());
//! let router = LayerRouter::new()
//!     .with(files_service_rpc_service_descriptor(), architect_files::serve(files.clone()))
//!     .merge(architect_files::stream_layer(files.clone()));
//! let gate = PermissionsGate::new(engine, identity)
//!     .permit(files_service_rpc_service_descriptor(), &architect_files::PERMITS)
//!     .permit(files_service_stream_service_descriptor(), &architect_files::STREAM_PERMITS);
//! // per connection, at `/org/{slug}/vox`:
//! let handler = confine(org, PermissionsGate::wrap_shared_with_bearer(gate, router, bearer));
//! ```
//!
//! In process (a server's own job, a test), [`FilesServer::acting`] is
//! the same service for an explicit org and principal.
//!
//! **Renditions** ([`FilesServer::with_renditions`], `renditions.rs`):
//! every version that lands schedules its media's renditions, made by
//! the [`Transcoder`] given (`FfmpegTranscoder`, feature `ffmpeg`) off
//! the request path, kept under `<data>/renditions/`.
//!
//! **The byte face** (feature `http`, `WebFiles`): the same service for
//! a browser, over `crate::web`'s routes, the caller resolved from the
//! request's session by the gate's own `IdentityResolver`, or from a
//! read ticket ([`crate::FilesService::ticket`]) signed with this
//! server's key ([`FilesServer::ticket_key`]).
//!
//! Grants can come from elsewhere too: [`FilesServer::with_grants`] adds
//! a [`GrantSource`] (a space's members, `architect-spaces`), asked on
//! every call, so a change there binds on the next call here.

mod acting;
#[cfg(feature = "ffmpeg")]
mod ffmpeg;
mod renditions;
mod state;
mod ticket;
#[cfg(feature = "http")]
mod web;

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use architect::PubSub;
use architect::blob::LocalFs;
use architect_orgs::OrgId;
use architect_orgs::server::Members;
use architect_permissions::Principal;

use crate::access::{ALL, Access, Capability, Grant, resolve, role_capabilities, stewards};
use crate::error::FilesFault;
use crate::id::RootId;
use crate::model::{FilesEvent, Usage};
use crate::path::RootPath;

pub use acting::Acting;
#[cfg(feature = "ffmpeg")]
pub use ffmpeg::FfmpegTranscoder;
pub use renditions::{Renditions, TranscodeError, Transcoder};
use state::State;
pub use ticket::TICKET_TTL_US;
use ticket::TicketKey;
#[cfg(feature = "http")]
pub use web::WebFiles;

/// How many events a slow subscriber may fall behind before it loses the
/// oldest.
const EVENT_BACKLOG: usize = 1024;

/// Grants a person holds in an org's files from somewhere other than
/// the files feature's own table: the folders a space covers, for its
/// members (`architect-spaces`).
///
/// Asked whenever a call needs to know what the person holds (each call,
/// each event of a subscription), so a membership removed there conveys
/// nothing on the next call here. Answer from memory: it runs on every
/// call.
pub trait GrantSource: Send + Sync {
    /// The live grants `user` holds in `org`'s files at `now_us`.
    fn grants(&self, org: &OrgId, user: &str, now_us: i64) -> Vec<Grant>;
}

/// Every org's files.
#[derive(Clone)]
pub struct FilesServer {
    inner: Arc<Inner>,
    default_org: Option<OrgId>,
    renditions: Option<Renditions>,
    tickets: TicketKey,
    granted: Option<Arc<dyn GrantSource>>,
}

struct Inner {
    data: PathBuf,
    blobs: LocalFs,
    members: Members,
    orgs: Mutex<HashMap<OrgId, Arc<OrgFiles>>>,
}

/// One org's files, loaded.
pub(crate) struct OrgFiles {
    dir: PathBuf,
    state: tokio::sync::Mutex<State>,
    hub: PubSub<FilesEvent>,
}

/// Who a call acts for, in its org.
#[derive(Debug, Clone)]
pub(crate) struct Actor {
    /// Their user id; `None` for an in-process service.
    user: Option<String>,
    /// What versions and roots record as their author.
    label: String,
    /// What their role holds everywhere in the org.
    baseline: Vec<Capability>,
    /// Whether they keep the org's roots.
    steward: bool,
    /// Their grants from a [`GrantSource`], as of this call.
    granted: Vec<Grant>,
}

impl FilesServer {
    /// Files under `data` (`<data>/org/<id>/files/`), bytes in `blobs`,
    /// roles from `members` (the gate's `MembersResolver` fills it).
    pub fn new(data: impl Into<PathBuf>, blobs: LocalFs, members: Members) -> Self {
        Self {
            inner: Arc::new(Inner {
                data: data.into(),
                blobs,
                members,
                orgs: Mutex::new(HashMap::new()),
            }),
            default_org: None,
            renditions: None,
            tickets: TicketKey::random(),
            granted: None,
        }
    }

    /// Make renditions of every media version with `transcoder`, one at
    /// a time, recorded under `<data>/renditions/` with their bytes in
    /// the blob store. Without it a rendition is
    /// [`FilesFault::NoRendition`].
    #[must_use]
    pub fn with_renditions(self, transcoder: impl Transcoder) -> Self {
        let engine = Renditions::new(
            self.inner.data.join("renditions"),
            self.inner.blobs.clone(),
            transcoder,
        );
        self.with_rendition_engine(engine)
    }

    /// Renditions made by `engine` (one built with
    /// [`Renditions::concurrency`], or shared with another server).
    #[must_use]
    pub fn with_rendition_engine(mut self, engine: Renditions) -> Self {
        self.renditions = Some(engine);
        self
    }

    /// The renditions, if this server makes them: to build one now
    /// ([`Renditions::build`]) or ask after one in a job of the app's.
    #[must_use]
    pub const fn rendition_engine(&self) -> Option<&Renditions> {
        self.renditions.as_ref()
    }

    /// Sign read tickets with `key`, so every server sharing it (and this
    /// one after a restart) reads the others'. Without it the key is
    /// random, and tickets read until the server stops.
    #[must_use]
    pub const fn ticket_key(mut self, key: [u8; 32]) -> Self {
        self.tickets = TicketKey::new(key);
        self
    }

    /// Grants from `source` too, beside the org's own table: what a
    /// person holds is the union. Generated grants are not listed by
    /// `grants` and cannot be revoked here; they end where they come
    /// from.
    #[must_use]
    pub fn with_grants(mut self, source: impl GrantSource + 'static) -> Self {
        self.granted = Some(Arc::new(source));
        self
    }

    /// The org a call on a lane in no org is in: a server that serves one
    /// org. Without it such a call is [`FilesFault::NoOrg`].
    #[must_use]
    pub fn default_org(mut self, org: OrgId) -> Self {
        self.default_org = Some(org);
        self
    }

    /// The service for `who` in `org`, called directly.
    #[must_use]
    pub fn acting(&self, org: OrgId, who: Principal) -> Acting {
        Acting::new(self.clone(), org, who)
    }

    /// The service for the call running now: its lane's org (or the
    /// default) and the gate's principal.
    fn for_call(&self) -> Result<Acting, FilesFault> {
        let org = architect_orgs::server::current_org()
            .or_else(|| self.default_org.clone())
            .ok_or(FilesFault::NoOrg)?;
        let who = architect::permissions_gate::caller().ok_or(FilesFault::Unauthenticated)?;
        Ok(self.acting(org, who))
    }

    /// Bound the bytes `org`'s live files may hold (`None`: unbounded).
    /// The deployment's call, not a member's.
    pub async fn set_quota(&self, org: &OrgId, quota: Option<u64>) -> Result<Usage, FilesFault> {
        let files = self.org(org).await?;
        let mut state = files.state.lock().await;
        let mut next = state.clone();
        next.quota = quota;
        files.save(&next).await?;
        *state = next;
        Ok(Usage {
            used: state.used(),
            quota,
        })
    }

    /// `org`'s files, loaded from disk the first time.
    pub(crate) async fn org(&self, org: &OrgId) -> Result<Arc<OrgFiles>, FilesFault> {
        if let Some(files) = self.loaded(org) {
            return Ok(files);
        }
        let dir = self.inner.data.join(org.path("files"));
        let state = load(&dir).await?;
        let mut orgs = self
            .inner
            .orgs
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Ok(Arc::clone(orgs.entry(org.clone()).or_insert_with(|| {
            Arc::new(OrgFiles {
                dir,
                state: tokio::sync::Mutex::new(state),
                hub: PubSub::sliding(EVENT_BACKLOG),
            })
        })))
    }

    fn loaded(&self, org: &OrgId) -> Option<Arc<OrgFiles>> {
        self.inner
            .orgs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(org)
            .cloned()
    }

    pub(crate) fn blobs(&self) -> &LocalFs {
        &self.inner.blobs
    }

    pub(crate) const fn tickets(&self) -> &TicketKey {
        &self.tickets
    }

    /// Whether `who` holds a role in `org` (a service is in every org).
    pub(crate) fn is_member(&self, org: &OrgId, who: &Principal) -> bool {
        matches!(who, Principal::Service { .. }) || self.inner.members.role(who, org).is_some()
    }

    /// Who `who` is in `org`, for the files feature.
    pub(crate) fn actor(&self, org: &OrgId, who: &Principal) -> Result<Actor, FilesFault> {
        match who {
            Principal::Service { name } => Ok(Actor {
                user: None,
                label: format!("service:{name}"),
                baseline: ALL.to_vec(),
                steward: true,
                granted: Vec::new(),
            }),
            Principal::User { user_id } => {
                let role = self.inner.members.role(who, org);
                Ok(Actor {
                    user: Some(user_id.clone()),
                    label: user_id.clone(),
                    baseline: role
                        .as_ref()
                        .map(|r| role_capabilities(r).to_vec())
                        .unwrap_or_default(),
                    steward: role.as_ref().is_some_and(stewards),
                    granted: self
                        .granted
                        .as_ref()
                        .map(|source| source.grants(org, user_id, now_us()))
                        .unwrap_or_default(),
                })
            }
            Principal::Guest { .. } | Principal::Host { .. } | Principal::Anonymous => {
                Err(FilesFault::Unauthenticated)
            }
        }
    }

    /// What `event` a subscriber acting as `actor` may hear, narrowed to
    /// what it may see (`None`: nothing of it).
    fn visible(
        actor: &Actor,
        state: &State,
        only: Option<RootId>,
        event: FilesEvent,
    ) -> Option<FilesEvent> {
        let now = now_us();
        let grants = actor.grants(state);
        let at = |root: RootId, path: &RootPath| {
            resolve(
                &actor.baseline,
                &grants,
                actor.user.as_deref(),
                root,
                path,
                now,
            )
        };
        let in_scope = |root: RootId| only.is_none_or(|o| o == root);
        match event {
            FilesEvent::Attached => Some(FilesEvent::Attached),
            FilesEvent::RootCreated(ref root) | FilesEvent::RootRenamed(ref root) => {
                (in_scope(root.id) && actor.sees_root(state, root.id, now)).then_some(event)
            }
            FilesEvent::RootReleased(id) => {
                (in_scope(id) && !actor.baseline.is_empty()).then_some(event)
            }
            FilesEvent::Changed { root_id, entries } => {
                let entries: Vec<_> = entries
                    .into_iter()
                    .filter(|e| at(root_id, &e.path).allows(Capability::Read))
                    .collect();
                (in_scope(root_id) && !entries.is_empty())
                    .then_some(FilesEvent::Changed { root_id, entries })
            }
            FilesEvent::Removed { root_id, paths } => {
                let paths: Vec<_> = paths
                    .into_iter()
                    .filter(|p| at(root_id, p).is_visible())
                    .collect();
                (in_scope(root_id) && !paths.is_empty())
                    .then_some(FilesEvent::Removed { root_id, paths })
            }
            FilesEvent::Labelled(ref l) | FilesEvent::Unlabelled(ref l) => (in_scope(l.root_id)
                && at(l.root_id, &l.path).allows(Capability::History))
            .then_some(event),
            FilesEvent::Granted(ref g) | FilesEvent::Revoked(ref g) => (in_scope(g.root_id)
                && (actor.user.as_deref() == Some(g.user.as_str())
                    || at(g.root_id, &g.path).allows(Capability::Share)))
            .then_some(event),
        }
    }
}

impl Actor {
    /// Every grant that may apply to this actor: the org's table and
    /// their generated ones.
    fn grants<'s>(&self, state: &'s State) -> Cow<'s, [Grant]> {
        if self.granted.is_empty() {
            Cow::Borrowed(&state.grants)
        } else {
            Cow::Owned(state.grants.iter().chain(&self.granted).cloned().collect())
        }
    }

    /// What this actor holds at `path`.
    pub(crate) fn at(&self, state: &State, root: RootId, path: &RootPath) -> Access {
        resolve(
            &self.baseline,
            &self.grants(state),
            self.user.as_deref(),
            root,
            path,
            now_us(),
        )
    }

    /// Refuse unless this actor may do `cap` at `path`: a path they hold
    /// nothing over is absent, one they may see and not do this at is
    /// denied.
    pub(crate) fn need(
        &self,
        state: &State,
        root: RootId,
        path: &RootPath,
        cap: Capability,
    ) -> Result<(), FilesFault> {
        state.root(root)?;
        match self.at(state, root, path) {
            Access::Absent => Err(FilesFault::PathNotFound(path.clone())),
            access if access.allows(cap) => Ok(()),
            Access::Holds(_) => Err(FilesFault::denied(format!("{cap:?}"), path.clone())),
        }
    }

    /// Whether this actor can name `root` at all: a role in the org, or a
    /// grant in it.
    pub(crate) fn sees_root(&self, state: &State, root: RootId, now: i64) -> bool {
        !self.baseline.is_empty()
            || self.grants(state).iter().any(|g| {
                g.root_id == root && self.user.as_deref() == Some(g.user.as_str()) && g.is_live(now)
            })
    }
}

impl OrgFiles {
    /// Write `state` atomically: a temporary file renamed over the last.
    async fn save(&self, state: &State) -> Result<(), FilesFault> {
        let json = facet_json::to_string(state).map_err(|e| FilesFault::Internal(e.to_string()))?;
        tokio::fs::create_dir_all(&self.dir).await.map_err(io)?;
        let path = self.dir.join(STATE_FILE);
        let tmp = self.dir.join(format!("{STATE_FILE}.tmp"));
        tokio::fs::write(&tmp, json).await.map_err(io)?;
        tokio::fs::rename(&tmp, &path).await.map_err(io)
    }

    /// Where an upload's bytes wait.
    fn staging(&self, upload: crate::id::UploadId) -> PathBuf {
        self.dir.join("uploads").join(format!("{upload}.part"))
    }
}

const STATE_FILE: &str = "state.json";

async fn load(dir: &Path) -> Result<State, FilesFault> {
    match tokio::fs::read_to_string(dir.join(STATE_FILE)).await {
        Ok(json) => facet_json::from_str(&json)
            .map_err(|e| FilesFault::Internal(format!("{}: {e}", dir.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(io(e)),
    }
}

/// Microseconds since the Unix epoch.
pub(crate) fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_micros()).unwrap_or(i64::MAX))
}

pub(crate) fn io(e: impl std::fmt::Display) -> FilesFault {
    FilesFault::Io(e.to_string())
}
