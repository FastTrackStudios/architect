//! The files of a project this node holds, for an org's signed-in members
//! (`architect_files::held`).
//!
//! A vault's folder, or a held root's, listed with size and sha256 and
//! read by range over HTTP, under the member's rights on the org's lane.
//!
//! Served at `/org/{slug}/held/…` beside each hosted org's lane
//! (`/org/{slug}/vox`), so every proxy that routes the lane routes it too
//! (the chart's `node.paths` `/org`, the web app's dev proxy). A caller is
//! who the lane's gate says its bearer is (the issuer's access token the
//! web app holds, resolved by the issuer and kept for the lane's
//! re-check interval): a member of the org reads what their role lets
//! them read there (`OrgLane::reads_at`, the org's engine every other read
//! on the lane asks), a person of another org, a share link's guest and
//! anyone without a bearer nothing. A listing hands each file a URL with a
//! read ticket for the folder it listed, a ticket call one for a file: a
//! ticket proves who minted it, for what, until when ([`TICKET_TTL_US`]);
//! what that person may read is asked again on every use.
//!
//! A vault's files are read in place, never through a symlink, never a
//! hidden one (`.architect/`); their sha256 is kept per size and mtime, so
//! listing a folder of stems hashes each once. A held root's files are its
//! manifest's live versions, their bytes the store's by content.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use architect::host::EngineHost;
use architect::http::{ByteBody, RequestHead};
use architect::layer::Layer as _;
use architect_files::held::{self, HeldFile, HeldFiles, HeldListing, HeldTicket};
use architect_files::server::{TicketKey, ticket_principal, ticket_subject};
use architect_files::sync::Engine;
use architect_files::{FilesFault, RootPath};
use architect_permissions::Principal;
use facet::Facet;
use tracing::Instrument as _;
use tracing::field::Empty;
use uuid::Uuid;
use vault_live::Backend;

use super::org::{OrgLane, VaultOrgs};

/// How long a held file's ticket reads: an hour, long enough to stream a
/// song's stems through a session, short enough that one left in a
/// history or a log stops working the same morning. Ask again after.
pub const TICKET_TTL_US: i64 = 60 * 60 * 1_000_000;

/// A held file's ticket: `fht1.<payload>.<mac>` (`TicketKey::seal`).
const PREFIX: &str = "fht1.";

/// What a held file's ticket says.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
struct Claims {
    /// The org's id.
    org: String,
    place: String,
    /// The file, or the folder it reads everything under.
    path: String,
    /// `user:<id>`.
    who: String,
    expires_us: i64,
}

/// Where a place is.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Place {
    Vault(String),
    Root(Uuid),
}

impl Place {
    fn parse(place: &str) -> Option<Self> {
        if let Some(vault) = place.strip_prefix(held::VAULT) {
            return (!vault.is_empty()).then(|| Self::Vault(vault.to_owned()));
        }
        place
            .strip_prefix(held::ROOT)
            .and_then(|id| Uuid::parse_str(id).ok())
            .map(Self::Root)
    }

    const fn kind(&self) -> &'static str {
        match self {
            Self::Vault(_) => "vault",
            Self::Root(_) => "root",
        }
    }

    /// The resource the org's engine is asked about, org-relative.
    fn resource(&self, path: &str) -> String {
        match self {
            Self::Vault(vault) => format!("vaults/{vault}/{path}"),
            Self::Root(id) => format!("files/{id}/{path}"),
        }
    }
}

/// A vault file's hash, for its size and mtime.
#[derive(Debug, Clone)]
struct Hashed {
    size: u64,
    mtime_ms: i64,
    sha256: String,
}

/// One file found: path, size, sha256.
type Found = (String, u64, Option<String>);

struct Inner {
    lanes: Vec<OrgLane>,
    vaults: VaultOrgs,
    backend: Backend,
    engine: Option<Engine>,
    key: TicketKey,
    hashes: Mutex<HashMap<(String, String), Hashed>>,
}

/// [`HeldFiles`] on this node: its hosted orgs' vaults and held roots.
#[derive(Clone)]
pub struct HeldFace(Arc<Inner>);

impl HeldFace {
    /// The face over `lanes` (the orgs this node hosts, with their
    /// vaults), `vaults` (whose each vault is), `backend` (the vaults on
    /// disk) and `engine` (the held roots, when the node runs one).
    #[must_use]
    pub fn new(
        lanes: Vec<OrgLane>,
        vaults: VaultOrgs,
        backend: Backend,
        engine: Option<Engine>,
    ) -> Self {
        Self(Arc::new(Inner {
            lanes,
            vaults,
            backend,
            engine,
            key: TicketKey::random(),
            hashes: Mutex::new(HashMap::new()),
        }))
    }

    /// Serves it on `host`.
    #[must_use]
    pub fn mount(self, host: EngineHost) -> EngineHost {
        host.plugin(architect::layers![held::Service].provide_http(&self))
    }

    fn lane(&self, slug: &str) -> Result<&OrgLane, FilesFault> {
        self.0
            .lanes
            .iter()
            .find(|l| l.org.slug == slug)
            .ok_or(FilesFault::NoOrg)
    }

    /// The place named, if it is `lane`'s org's: one of its vaults, or a
    /// root this node holds filed under its slug.
    fn place(&self, lane: &OrgLane, place: &str) -> Result<Place, FilesFault> {
        let nowhere = || FilesFault::PathNotFound(RootPath::root());
        let found = Place::parse(place).ok_or_else(nowhere)?;
        let ours = match &found {
            Place::Vault(vault) => self.0.vaults.holds(&lane.org.id, vault),
            Place::Root(id) => self
                .0
                .engine
                .as_ref()
                .and_then(|e| e.meta(*id))
                .is_some_and(|m| m.org == lane.org.slug),
        };
        if ours { Ok(found) } else { Err(nowhere()) }
    }

    /// The person `token` (a bearer, never a ticket) is, a member of the
    /// lane's org.
    async fn person(&self, lane: &OrgLane, token: &str) -> Result<Principal, FilesFault> {
        let token = token.trim();
        if token.is_empty() || token.starts_with(PREFIX) {
            tracing::Span::current().record("auth.outcome", "absent");
            return Err(FilesFault::Unauthenticated);
        }
        let who = lane.person(Some(token)).await;
        if matches!(who, Principal::Anonymous) {
            tracing::Span::current().record("auth.outcome", "rejected");
            return Err(FilesFault::Unauthenticated);
        }
        tracing::Span::current().record("auth.outcome", "resolved");
        if !lane.is_member(&who) {
            return Err(denied());
        }
        Ok(who)
    }

    /// Who `ticket` is for, when this node signed it for `place` of the
    /// lane's org, it covers `path` and has not run out.
    fn ticket_holder(
        &self,
        lane: &OrgLane,
        ticket: &str,
        place: &str,
        path: &str,
    ) -> Option<Principal> {
        let claims: Claims = self.0.key.unseal(PREFIX, ticket)?;
        let fits = claims.expires_us > now_us()
            && claims.org == lane.org.id.0
            && claims.place == place
            && held::is_within(path, &claims.path);
        fits.then(|| ticket_principal(&claims.who)).flatten()
    }

    /// A ticket for `who` reading `path` (and under it) in `place`.
    fn mint(
        &self,
        lane: &OrgLane,
        who: &Principal,
        place: &str,
        path: &str,
    ) -> Option<(String, i64)> {
        let expires_us = now_us().saturating_add(TICKET_TTL_US);
        let claims = Claims {
            org: lane.org.id.0.clone(),
            place: place.to_owned(),
            path: path.to_owned(),
            who: ticket_subject(who)?,
            expires_us,
        };
        Some((self.0.key.seal(PREFIX, &claims)?, expires_us))
    }

    /// Every file under `folder` of `place`.
    async fn files(&self, place: &Place, folder: &str) -> Result<Vec<Found>, FilesFault> {
        match place {
            Place::Vault(vault) => {
                let (this, vault, folder) = (self.clone(), vault.clone(), folder.to_owned());
                tokio::task::spawn_blocking(move || this.vault_files(&vault, &folder))
                    .await
                    .map_err(|e| FilesFault::Internal(e.to_string()))?
            }
            Place::Root(id) => {
                let engine = self.0.engine.as_ref().ok_or(FilesFault::NoOrg)?;
                let tree = engine
                    .tree(*id)
                    .await
                    .map_err(|e| FilesFault::Io(e.to_string()))?;
                Ok(tree
                    .live()
                    .into_iter()
                    .filter(|(p, _)| {
                        held::is_within(p, folder)
                            && held::clean_path(p).as_deref() == Some(p.as_str())
                    })
                    .map(|(p, v)| (p, v.size, v.content))
                    .collect())
            }
        }
    }

    /// A vault folder's files, hashing only those not hashed at this size
    /// and mtime before.
    fn vault_files(&self, vault: &str, folder: &str) -> Result<Vec<Found>, FilesFault> {
        let root = self
            .0
            .backend
            .root(vault)
            .map_err(|_| FilesFault::PathNotFound(RootPath::root()))?;
        let known: Mutex<Vec<Found>> = Mutex::new(Vec::new());
        let read = self
            .0
            .backend
            .manifest_where(vault, |rel| {
                if !held::is_within(rel, folder) || held::clean_path(rel).as_deref() != Some(rel) {
                    return false;
                }
                match self.known(vault, &root, rel) {
                    Some(h) => {
                        known.lock().unwrap_or_else(PoisonError::into_inner).push((
                            rel.to_owned(),
                            h.size,
                            Some(h.sha256),
                        ));
                        false
                    }
                    None => true,
                }
            })
            .map_err(|e| FilesFault::Io(e.to_string()))?;
        let mut files = known.into_inner().unwrap_or_else(PoisonError::into_inner);
        {
            let mut kept = self.0.hashes.lock().unwrap_or_else(PoisonError::into_inner);
            for entry in &read.files {
                kept.insert(
                    (vault.to_owned(), entry.path.clone()),
                    Hashed {
                        size: entry.size,
                        mtime_ms: entry.mtime_ms,
                        sha256: entry.sha256.clone(),
                    },
                );
            }
        }
        files.extend(
            read.files
                .into_iter()
                .map(|entry| (entry.path, entry.size, Some(entry.sha256))),
        );
        files.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(files)
    }

    /// The hash kept for `rel` when the file is still that size and mtime.
    fn known(&self, vault: &str, root: &Path, rel: &str) -> Option<Hashed> {
        let meta = std::fs::symlink_metadata(root.join(rel)).ok()?;
        let kept = self
            .0
            .hashes
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&(vault.to_owned(), rel.to_owned()))
            .cloned()?;
        (meta.is_file() && meta.len() == kept.size && mtime_ms(&meta) == kept.mtime_ms)
            .then_some(kept)
    }

    /// Where the bytes of `path` in `place` are on this node.
    async fn bytes_at(&self, place: &Place, path: &str) -> Result<PathBuf, FilesFault> {
        let missing = || FilesFault::PathNotFound(root_path(path));
        match place {
            Place::Vault(vault) => {
                let root = self.0.backend.root(vault).map_err(|_| missing())?;
                own_file(&root, path).ok_or_else(missing)
            }
            Place::Root(id) => {
                let engine = self.0.engine.as_ref().ok_or_else(missing)?;
                let tree = engine.tree(*id).await.map_err(|_| missing())?;
                let content = tree
                    .current(path)
                    .and_then(|v| v.content)
                    .ok_or_else(missing)?;
                let hash = architect::blob::Sha256::from_hex(&content).map_err(|_| missing())?;
                engine.store.file_for(&hash).ok_or_else(|| {
                    FilesFault::Io("not on this node now: try again later".to_owned())
                })
            }
        }
    }
}

impl HeldFiles for HeldFace {
    async fn list(
        &self,
        slug: String,
        place: String,
        folder: Option<String>,
        token: String,
    ) -> Result<HeldListing, FilesFault> {
        let span = span("list", &slug);
        async {
            let lane = self.lane(&slug)?;
            let found = self.place(lane, &place)?;
            span_kind(&found);
            let who = self.person(lane, &token).await?;
            let folder = held::clean_path(folder.as_deref().unwrap_or_default())
                .ok_or_else(|| FilesFault::invalid("not a folder of the place"))?;
            let (ticket, expires_us) = self
                .mint(lane, &who, &place, &folder)
                .ok_or(FilesFault::Unauthenticated)?;
            let files: Vec<HeldFile> = self
                .files(&found, &folder)
                .await?
                .into_iter()
                .filter(|(p, _, _)| lane.reads_at(&who, &found.resource(p)))
                .map(|(path, size, sha256)| HeldFile {
                    url: held::content_url("", &slug, &place, &path, Some(&ticket)),
                    path,
                    size,
                    sha256,
                })
                .collect();
            tracing::Span::current().record("held.files", files.len());
            Ok(HeldListing {
                place: place.clone(),
                folder,
                files,
                expires_us,
            })
        }
        .instrument(span.clone())
        .await
        .inspect(|_| {
            span.record("held.outcome", "listed");
        })
        .inspect_err(|e| refused(&span, e))
    }

    async fn ticket(
        &self,
        slug: String,
        place: String,
        path: String,
        token: String,
    ) -> Result<HeldTicket, FilesFault> {
        let span = span("ticket", &slug);
        async {
            let lane = self.lane(&slug)?;
            let found = self.place(lane, &place)?;
            span_kind(&found);
            let who = self.person(lane, &token).await?;
            let path = held::clean_path(&path)
                .filter(|p| !p.is_empty())
                .ok_or_else(|| FilesFault::PathNotFound(root_path(&path)))?;
            if !lane.reads_at(&who, &found.resource(&path)) {
                return Err(FilesFault::PathNotFound(root_path(&path)));
            }
            // Only a file that is there: a ticket names nothing else.
            self.bytes_at(&found, &path).await?;
            let (ticket, expires_us) = self
                .mint(lane, &who, &place, &path)
                .ok_or(FilesFault::Unauthenticated)?;
            Ok(HeldTicket {
                url: held::content_url("", &slug, &place, &path, Some(&ticket)),
                expires_us,
            })
        }
        .instrument(span.clone())
        .await
        .inspect(|_| {
            span.record("held.outcome", "minted");
        })
        .inspect_err(|e| refused(&span, e))
    }

    async fn content(
        &self,
        slug: String,
        place: String,
        path: String,
        head: RequestHead,
    ) -> Result<ByteBody, FilesFault> {
        let span = span("content", &slug);
        async {
            let lane = self.lane(&slug)?;
            let found = self.place(lane, &place)?;
            span_kind(&found);
            let path = held::clean_path(&path)
                .filter(|p| !p.is_empty())
                .ok_or_else(|| FilesFault::PathNotFound(root_path(&path)))?;
            // A ticket from the query, or the bearer from its header; a
            // session is never read from the URL.
            let who = if let Some(ticket) = head.query_param("ticket") {
                tracing::Span::current().record("held.via", "ticket");
                self.ticket_holder(lane, &ticket, &place, &path)
                    .ok_or(FilesFault::Unauthenticated)?
            } else {
                tracing::Span::current().record("held.via", "bearer");
                self.person(lane, head.bearer().unwrap_or_default()).await?
            };
            if !lane.is_member(&who) || !lane.reads_at(&who, &found.resource(&path)) {
                return Err(FilesFault::PathNotFound(root_path(&path)));
            }
            let file = self.bytes_at(&found, &path).await?;
            let body = ByteBody::file(&file, held::content_type(&path))
                .await
                .map_err(|_| FilesFault::PathNotFound(root_path(&path)))?;
            Ok(body)
        }
        .instrument(span.clone())
        .await
        .inspect(|_| {
            span.record("held.outcome", "served");
        })
        .inspect_err(|e| refused(&span, e))
    }
}

/// The request's span: the wide event its fields fill. No path or token
/// in it, only their shapes.
fn span(op: &'static str, slug: &str) -> tracing::Span {
    tracing::info_span!(
        "held_files",
        otel.name = %format!("HeldFiles/{op}"),
        held.op = op,
        org.slug = %slug,
        held.place_kind = Empty,
        held.via = Empty,
        held.files = Empty,
        auth.outcome = Empty,
        held.outcome = Empty,
    )
}

fn span_kind(place: &Place) {
    tracing::Span::current().record("held.place_kind", place.kind());
}

/// A refusal is one line (alertable); anything else rides the span.
fn refused(span: &tracing::Span, e: &FilesFault) {
    let outcome = match e {
        FilesFault::Unauthenticated => "unauthenticated",
        FilesFault::Denied { .. } => "denied",
        FilesFault::NoOrg | FilesFault::PathNotFound(_) => "not_found",
        _ => "failed",
    };
    span.record("held.outcome", outcome);
    if matches!(e, FilesFault::Unauthenticated | FilesFault::Denied { .. }) {
        span.in_scope(|| tracing::warn!(held.outcome = outcome, "held files: refused"));
    }
}

fn denied() -> FilesFault {
    FilesFault::denied("read this org's files (not a member)", RootPath::root())
}

fn root_path(path: &str) -> RootPath {
    RootPath::parse(path).unwrap_or_else(|_| RootPath::root())
}

/// `rel` under `root` when it is a plain file of it: reached through no
/// symlink (the resolved path is the joined one) and inside it.
fn own_file(root: &Path, rel: &str) -> Option<PathBuf> {
    let rel = held::clean_path(rel).filter(|r| !r.is_empty())?;
    let root = root.canonicalize().ok()?;
    let joined = root.join(&rel);
    let resolved = joined.canonicalize().ok()?;
    (resolved == joined && resolved.is_file()).then_some(resolved)
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_micros()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::{Place, own_file};

    #[test]
    fn a_place_is_a_vault_or_a_root_by_id() {
        assert_eq!(
            Place::parse("vault:notes"),
            Some(Place::Vault("notes".into()))
        );
        assert_eq!(Place::parse("vault:"), None);
        assert_eq!(Place::parse("notes"), None);
        let id = uuid::Uuid::nil();
        assert_eq!(Place::parse(&format!("root:{id}")), Some(Place::Root(id)));
        assert_eq!(Place::parse("root:nope"), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_vault_file_is_read_through_no_symlink() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("vault");
        std::fs::create_dir_all(root.join("Songs")).unwrap();
        std::fs::write(root.join("Songs/a.wav"), b"a").unwrap();
        std::fs::write(tmp.path().join("secret"), b"s").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("secret"), root.join("Songs/link.wav")).unwrap();
        std::os::unix::fs::symlink(root.join("Songs"), root.join("Alias")).unwrap();
        assert!(own_file(&root, "Songs/a.wav").is_some());
        assert!(own_file(&root, "Songs/link.wav").is_none());
        assert!(own_file(&root, "Alias/a.wav").is_none());
        assert!(own_file(&root, "Songs/../../secret").is_none());
        assert!(own_file(&root, "Songs").is_none());
    }
}
