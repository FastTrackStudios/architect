//! The engine: each root's working copy and its manifest kept in step.
//!
//! One [`Engine::pass`] per root:
//!
//! 1. **Scan** the working copy (pinned roots only) and append a version
//!    per settled change. Capture comes before applying, so a pull never
//!    wipes a change that was only on disk (Task's trap).
//! 2. **Apply** the manifest's heads: fetch what is missing (from this
//!    device's store, any working file holding the same bytes, else a
//!    peer through the [`Fetcher`]), write it beside the file and rename
//!    it into place, delete what was deleted. A file whose stat differs
//!    from the index has a change pending and is never overwritten.
//! 3. **Settle forks**: the newest content wins the path, every other
//!    head is written beside it as a conflict copy, and one version
//!    replacing all heads closes the fork.
//! 4. **Checkpoint**: the first import, and every pass that saved a
//!    session file (a save point).
//!
//! The documents come from the app ([`Docs`]: the node's registry), each
//! pass anew, so an evicted document is never held past its registry.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use architect::blob::{Fetcher, Sha256};
use facet::Facet;
use futures_util::future::BoxFuture;
use loro::LoroDoc;
use uuid::Uuid;

use super::layout::{self, PART};
use super::manifest::{
    Checkpoint, FileVersion, Manifest, RootMeta, Section, Settled, Tree, conflict_path,
};
use super::scan::{Index, Indexed, Scanner, Seen, Stat};
use super::store::TreeStore;
use crate::id::VersionId;
pub use crate::roots::RootStatus;

/// Committed files up to this size are also kept in the store.
///
/// So every version of them is kept; larger ones stay in place. A
/// device's default; a node that holds its tree in place keeps none
/// (`0`): its tree is the copy, and a second one in the store is what
/// filled a disk (#525).
pub const KEEP_INLINE: u64 = 64 << 20;

/// A root's document, from the app's registry.
pub type Docs = Arc<dyn Fn(Uuid) -> BoxFuture<'static, Option<LoroDoc>> + Send + Sync>;

/// How the engine is set up on a device.
#[derive(Clone)]
pub struct EngineConfig {
    /// The device's name: a version's `by`, a conflict copy's suffix.
    pub device: String,
    /// The agent's own state: indexes, pins.
    pub data: PathBuf,
    /// Where roots are: on the node, the tree it serves in place (every
    /// root found there is held); on a laptop, where pins go by default.
    pub files_dir: PathBuf,
    /// Whether every root found in `files_dir` is held (the node).
    pub hold_all: bool,
    /// Committed files up to this size are also copied into the store
    /// ([`KEEP_INLINE`]; `0` on a holding node), and fetched ones up to
    /// it stay there as well as in the working copy.
    pub keep_inline: u64,
    /// How old a change must be before it is committed ([`super::scan::SETTLE`]).
    pub settle: std::time::Duration,
    /// Where this device is (`home`, `starcommand`): copies at one site
    /// are not off-site. Empty: the device's name.
    pub site: String,
    /// How long a copy's evidence (a scrub, a passed challenge) counts.
    pub verify_window: std::time::Duration,
    /// Bytes re-hashed per pass at most (0: no scrubs between `FILES_SCRUB`s).
    pub scrub_budget: u64,
    /// Peers' copies challenged per pass.
    pub challenges: u32,
}

/// How long evidence of a copy counts by default: 30 days.
pub const VERIFY_WINDOW: std::time::Duration = std::time::Duration::from_hours(30 * 24);

impl EngineConfig {
    /// A device's engine with the defaults: [`KEEP_INLINE`] (`0` when
    /// `hold_all`: nothing stored twice), [`super::scan::SETTLE`].
    #[must_use]
    pub fn new(
        device: impl Into<String>,
        data: PathBuf,
        files_dir: PathBuf,
        hold_all: bool,
    ) -> Self {
        Self {
            device: device.into(),
            data,
            files_dir,
            hold_all,
            keep_inline: if hold_all { 0 } else { KEEP_INLINE },
            settle: super::scan::SETTLE,
            site: String::new(),
            verify_window: VERIFY_WINDOW,
            scrub_budget: 32 << 20,
            challenges: 2,
        }
    }
}

/// Why an engine call failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    #[error("no such root: {0}")]
    NoRoot(String),
    #[error("no such version")]
    NoVersion,
    #[error("not synced yet: {}", .0.join(", "))]
    NotSynced(Vec<String>),
    /// Dropping this device's copies would leave these files under policy.
    #[error("under the copy policy without this device: {}", .0.join(", "))]
    BelowPolicy(Vec<String>),
    #[error("{0}")]
    Io(String),
    /// The store's budget has no room: deferred until there is.
    #[error("{0}")]
    NoRoom(String),
}

impl From<architect::blob::BlobError> for EngineError {
    fn from(e: architect::blob::BlobError) -> Self {
        match e {
            architect::blob::BlobError::NoRoom(no) => Self::NoRoom(no.to_string()),
            other => Self::Io(other.to_string()),
        }
    }
}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

#[derive(Default)]
struct RootState {
    meta: Option<RootMeta>,
    /// Where it was found (held roots).
    found: Option<PathBuf>,
    scanner: Scanner,
    index: Option<Index>,
    missing: u32,
    /// Files whose fetch waits for room in the store.
    deferred: u32,
    error: Option<String>,
}

#[derive(Debug, Clone, Default, Facet)]
struct Pins {
    /// doc id → the working copy's path.
    roots: BTreeMap<String, String>,
}

/// The engine. Cheap to clone.
#[derive(Clone)]
pub struct Engine {
    config: Arc<EngineConfig>,
    pub store: TreeStore,
    pub fetcher: Fetcher,
    docs: Docs,
    roots: Arc<Mutex<BTreeMap<Uuid, RootState>>>,
    /// One pass per root at a time.
    busy: Arc<tokio::sync::Mutex<()>>,
    /// Hashes some manifest names: what the blob lane sends.
    pub(super) served: Arc<std::sync::RwLock<std::collections::HashSet<Sha256>>>,
    /// The manifests as the peer lane lists them.
    pub(super) held: Arc<std::sync::RwLock<Vec<super::lane::HeldManifest>>>,
    /// The drive's mount, and whether a peer answers: what `Files::drive` reports.
    pub(super) drive: Arc<std::sync::RwLock<(Option<PathBuf>, bool)>>,
    /// What makes peaks of anything but a WAV.
    pub(super) transcoder: Option<Arc<dyn crate::server::Transcoder>>,
    /// The registry, the ledgers, this device's attestations.
    pub(super) books: Arc<super::replicas::Books>,
}

impl Engine {
    /// An engine whose bytes are `store`'s, fetched through `fetcher`
    /// (over the same `LocalFs`), and whose documents come from `docs`.
    #[must_use]
    pub fn new(config: EngineConfig, store: TreeStore, fetcher: Fetcher, docs: Docs) -> Self {
        let books = Arc::new(super::replicas::Books::new(&config.data));
        Self {
            config: Arc::new(config),
            books,
            store,
            fetcher,
            docs,
            roots: Arc::default(),
            busy: Arc::default(),
            served: Arc::default(),
            held: Arc::default(),
            transcoder: None,
            drive: Arc::default(),
        }
    }

    /// Say where this device's drive is mounted (`None`: no drive).
    pub fn set_drive(&self, at: Option<PathBuf>) {
        self.drive.write().unwrap_or_else(PoisonError::into_inner).0 = at;
    }

    /// Say whether a peer answers the fetcher.
    pub fn set_online(&self, online: bool) {
        self.drive.write().unwrap_or_else(PoisonError::into_inner).1 = online;
    }

    /// The drive's mount and whether a peer answers.
    #[must_use]
    pub fn drive_info(&self) -> (Option<PathBuf>, bool) {
        self.drive
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Make peaks of non-WAV media with `transcoder` (`FfmpegTranscoder`).
    #[must_use]
    pub fn with_transcoder(mut self, transcoder: impl crate::server::Transcoder) -> Self {
        self.transcoder = Some(Arc::new(transcoder));
        self
    }

    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<Uuid, RootState>> {
        self.roots.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Find the roots in `files_dir` (on the node: all of them, held).
    pub fn discover(&self) {
        if !self.config.hold_all {
            return;
        }
        for f in layout::discover(&self.config.files_dir) {
            self.with_root(f.meta.doc_id(), |state| {
                state.meta = Some(f.meta);
                state.found = Some(f.dir);
            });
        }
    }

    /// Run `f` on a root's state (made if new), the lock held only for it.
    fn with_root<T>(&self, doc_id: Uuid, f: impl FnOnce(&mut RootState) -> T) -> T {
        f(self.lock().entry(doc_id).or_default())
    }

    /// A root another device named on the lane: kept from now on.
    pub fn learn(&self, doc_id: Uuid) {
        self.lock().entry(doc_id).or_default();
    }

    /// Every root this device knows.
    #[must_use]
    pub fn known(&self) -> Vec<Uuid> {
        self.lock().keys().copied().collect()
    }

    /// Whether `doc_id` is a root this device knows.
    #[must_use]
    pub fn knows(&self, doc_id: Uuid) -> bool {
        self.lock().contains_key(&doc_id) || self.knows_ledger(doc_id)
    }

    /// Stop keeping a root: its state here (its access was revoked).
    pub(super) fn forget_root(&self, doc_id: Uuid) {
        self.lock().remove(&doc_id);
    }

    /// A root's place, once known.
    #[must_use]
    pub fn meta(&self, doc_id: Uuid) -> Option<RootMeta> {
        self.lock().get(&doc_id).and_then(|s| s.meta.clone())
    }

    fn pins_path(&self) -> PathBuf {
        self.config.data.join("pins.json")
    }

    fn pins(&self) -> Pins {
        std::fs::read_to_string(self.pins_path())
            .ok()
            .and_then(|s| facet_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save_pins(&self, pins: &Pins) -> Result<(), EngineError> {
        std::fs::create_dir_all(&self.config.data)?;
        std::fs::write(
            self.pins_path(),
            facet_json::to_string(pins).unwrap_or_default(),
        )?;
        Ok(())
    }

    /// The working copy of `doc_id` on this device, if any.
    #[must_use]
    pub fn working_dir(&self, doc_id: Uuid) -> Option<PathBuf> {
        let found = self.lock().get(&doc_id).and_then(|s| s.found.clone());
        if found.is_some() {
            return found;
        }
        self.pins()
            .roots
            .get(&doc_id.to_string())
            .map(PathBuf::from)
    }

    fn index_path(&self, doc_id: Uuid) -> PathBuf {
        self.config
            .data
            .join("index")
            .join(format!("{doc_id}.json"))
    }

    /// The root's document.
    pub async fn doc(&self, doc_id: Uuid) -> Option<LoroDoc> {
        (self.docs)(doc_id).await
    }

    /// The root's versions, as a tree.
    pub async fn tree(&self, doc_id: Uuid) -> Result<Tree, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        Ok(Manifest::new(&doc).tree())
    }

    /// Pin a root: a full working copy at `path` (default
    /// `<files_dir>/<org>/<section>/<name>`), synced both ways from the
    /// next pass.
    ///
    /// # Errors
    ///
    /// The root is not known, or the pins cannot be written.
    pub async fn pin(&self, doc_id: Uuid, path: Option<PathBuf>) -> Result<PathBuf, EngineError> {
        let meta = self.root_meta(doc_id).await?;
        let path = path.unwrap_or_else(|| layout::dir_for(&self.config.files_dir, &meta));
        std::fs::create_dir_all(&path)?;
        let mut pins = self.pins();
        pins.roots
            .insert(doc_id.to_string(), path.display().to_string());
        self.save_pins(&pins)?;
        self.pass(doc_id).await;
        Ok(path)
    }

    /// Unpin a root: its working copy is removed, only when every file
    /// in it is synced. The manifest stays, so it is still browsable.
    ///
    /// # Errors
    ///
    /// [`EngineError::NotSynced`] names the files that are not.
    pub async fn unpin(&self, doc_id: Uuid) -> Result<(), EngineError> {
        let _busy = self.busy.lock().await;
        let mut pins = self.pins();
        let Some(dir) = pins.roots.get(&doc_id.to_string()).map(PathBuf::from) else {
            return Ok(());
        };
        let tree = self.tree(doc_id).await?;
        let index = Index::load(&self.index_path(doc_id));
        let live = tree.live();
        let mut unsynced = Vec::new();
        for (path, known) in &index.files {
            let stat = std::fs::metadata(dir.join(path)).ok().map(|m| Stat::of(&m));
            let at_head = live.get(path).is_some_and(|v| Some(v.id) == known.version);
            if stat != Some(known.stat) || !at_head {
                unsynced.push(path.clone());
            }
        }
        let mut stray = Vec::new();
        list_files(&dir, "", &mut stray);
        unsynced.extend(stray.into_iter().filter(|p| !index.files.contains_key(p)));
        if !unsynced.is_empty() {
            unsynced.sort();
            return Err(EngineError::NotSynced(unsynced));
        }
        if let Some(meta) = self.meta(doc_id) {
            let leaving = self.leaving_on_unpin(&meta.place(), &index);
            self.guard(&meta.org, leaving).await?;
        }
        for path in index.files.keys() {
            let _ = std::fs::remove_file(dir.join(path));
        }
        remove_empty_dirs(&dir);
        pins.roots.remove(&doc_id.to_string());
        self.save_pins(&pins)?;
        let _ = std::fs::remove_file(self.index_path(doc_id));
        if let Some(state) = self.lock().get_mut(&doc_id) {
            state.index = None;
            state.missing = 0;
        }
        // The working copy's attestations go now; what the cache keeps stays.
        if let Some(meta) = self.meta(doc_id) {
            self.attest_root(doc_id, &meta.org, &Index::default()).await;
            self.refresh_ledgers().await;
        }
        Ok(())
    }

    async fn root_meta(&self, doc_id: Uuid) -> Result<RootMeta, EngineError> {
        if let Some(meta) = self.meta(doc_id) {
            return Ok(meta);
        }
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        let meta = Manifest::new(&doc)
            .meta()
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        self.lock().entry(doc_id).or_default().meta = Some(meta.clone());
        Ok(meta)
    }

    /// One pass over every root this device knows.
    pub async fn pass_all(&self) {
        self.discover();
        for doc_id in self.known() {
            self.pass(doc_id).await;
        }
        self.refresh_lane().await;
        self.keep_books().await;
        self.refresh_lane().await;
    }

    /// A pass over every root every `every`, for as long as it runs.
    pub async fn run(self, every: std::time::Duration) {
        loop {
            self.pass_all().await;
            tokio::time::sleep(every).await;
        }
    }

    /// One pass over one root (see the module docs).
    pub async fn pass(&self, doc_id: Uuid) {
        let _busy = self.busy.lock().await;
        let result = self.pass_inner(doc_id).await;
        self.with_root(doc_id, |state| {
            state.error = result.err().map(|e| e.to_string());
        });
    }

    async fn pass_inner(&self, doc_id: Uuid) -> Result<(), EngineError> {
        let Some(doc) = self.doc(doc_id).await else {
            return Err(EngineError::NoRoot(doc_id.to_string()));
        };
        let manifest = Manifest::new(&doc);
        let found_meta = self.lock().get(&doc_id).and_then(|s| s.meta.clone());
        match (&found_meta, manifest.meta()) {
            (Some(found), None) => manifest.set_meta(found),
            (None, Some(meta)) => self.lock().entry(doc_id).or_default().meta = Some(meta),
            _ => {}
        }
        let Some(meta) = manifest.meta().or(found_meta) else {
            return Ok(());
        };
        let Some(dir) = self.working_dir(doc_id) else {
            // Not here: a library's notes are fetched anyway, so its
            // assets read offline.
            if meta.section == Section::Assets {
                self.fetch_notes(&manifest.tree()).await;
            }
            return Ok(());
        };
        let first_import = manifest.versions().is_empty();

        // 1. Scan.
        let index_file = self.index_path(doc_id);
        let (mut scanner, mut index) = self.with_root(doc_id, |state| {
            (
                std::mem::take(&mut state.scanner),
                state
                    .index
                    .take()
                    .unwrap_or_else(|| Index::load(&index_file)),
            )
        });
        for (path, known) in &index.files {
            if let Ok(hash) = Sha256::from_hex(&known.content) {
                self.store.in_place.insert(hash, dir.join(path), known.stat);
            }
        }
        let scan_dir = dir.clone();
        let settle = self.config.settle;
        let (scanner_back, index_back, seen) = tokio::task::spawn_blocking(move || {
            let seen = scanner.scan(&scan_dir, &mut index, SystemTime::now(), settle);
            (scanner, index, seen)
        })
        .await
        .map_err(|e| EngineError::Io(e.to_string()))?;
        (scanner, index) = (scanner_back, index_back);
        let outcome = match seen {
            Ok(seen) => self.commit(&manifest, &dir, &mut index, seen).await,
            Err(e) => Err(EngineError::Io(e.to_string())),
        };
        let saved = match outcome {
            Ok(saved) => saved,
            Err(e) => {
                self.put_back(doc_id, scanner, index, 0);
                return Err(e);
            }
        };

        // 2 and 3. Apply, settling forks.
        let (missing, deferred) = self.apply(&manifest, &dir, &mut index).await;
        self.note_deferred(doc_id, deferred);

        // 4. Checkpoint.
        if first_import && !index.files.is_empty() {
            self.checkpoint(manifest, "Imported", true);
        } else if let Some(session) = saved.iter().find(|p| layout::is_session_file(p)) {
            let name = session.rsplit('/').next().unwrap_or(session);
            self.checkpoint(manifest, &format!("Saved {name}"), true);
        }
        let _ = index.save(&self.index_path(doc_id));
        self.attest_root(doc_id, &meta.org, &index).await;
        self.put_back(doc_id, scanner, index, missing);
        Ok(())
    }

    /// Keep a root's count of fetches waiting for room; one event when
    /// a root starts waiting, not one per pass while it does.
    fn note_deferred(&self, doc_id: Uuid, deferred: u32) {
        let before = self.with_root(doc_id, |state| {
            std::mem::replace(&mut state.deferred, deferred)
        });
        if deferred > 0 && before == 0 {
            let usage = self.store.local.budget().map(|b| b.usage());
            tracing::warn!(
                files.root = %doc_id,
                files.deferred = deferred,
                store.used = usage.map(|u| u.used),
                store.max = usage.map(|u| u.max),
                fs.free = usage.and_then(|u| u.space).map(|s| s.free),
                fs.keep_free = usage.map(|u| u.keep_free),
                "files: fetches deferred for space"
            );
        }
    }

    /// Fetches waiting for room in the store, over every root.
    #[must_use]
    pub fn deferred(&self) -> u32 {
        self.lock()
            .values()
            .map(|s| s.deferred)
            .fold(0, u32::saturating_add)
    }

    fn put_back(&self, doc_id: Uuid, scanner: Scanner, index: Index, missing: u32) {
        self.with_root(doc_id, |state| {
            state.scanner = scanner;
            state.index = Some(index);
            state.missing = missing;
        });
    }

    /// Append a version per settled change; the paths saved.
    async fn commit(
        &self,
        manifest: &Manifest<'_>,
        dir: &Path,
        index: &mut Index,
        seen: Vec<Seen>,
    ) -> Result<Vec<String>, EngineError> {
        let tree = manifest.tree();
        let mut versions = Vec::new();
        let mut saved = Vec::new();
        for change in seen {
            match change {
                Seen::Changed {
                    path,
                    content,
                    stat,
                } => {
                    // Hashed just now by the scan: verified on arrival.
                    self.note_verified(&content);
                    let known = index.files.get(&path).and_then(|i| i.version);
                    let heads = tree.heads(&path);
                    // Already the head's bytes (a copy that was here before
                    // the pin, or a write this engine made): adopt it.
                    if let Some(head) = heads
                        .iter()
                        .find(|h| h.content.as_deref() == Some(&*content))
                    {
                        index.files.insert(
                            path,
                            Indexed {
                                stat,
                                content,
                                version: Some(head.id),
                            },
                        );
                        continue;
                    }
                    let base: Vec<VersionId> = match known {
                        Some(v) if tree.version(v).is_some() => vec![v],
                        _ => Vec::new(),
                    };
                    let hash =
                        Sha256::from_hex(&content).map_err(|e| EngineError::Io(e.to_string()))?;
                    let file = dir.join(&path);
                    // Kept in the store too, if small and there is room;
                    // else served from where it is.
                    let kept = stat.size <= self.config.keep_inline
                        && self.store.local.has_room(stat.size)
                        && match architect::blob::BlobStore::put(
                            &self.store.local,
                            Some(hash),
                            Box::pin(tokio::fs::File::open(&file).await?),
                        )
                        .await
                        {
                            Ok(_) => true,
                            Err(architect::blob::BlobError::NoRoom(_)) => false,
                            Err(e) => return Err(EngineError::Io(e.to_string())),
                        };
                    if !kept {
                        self.store.in_place.insert(hash, file, stat);
                    }
                    let version = FileVersion {
                        id: VersionId(Uuid::new_v4()),
                        path: path.clone(),
                        content: Some(content.clone()),
                        size: stat.size,
                        by: self.config.device.clone(),
                        at_us: stat.mtime_us(),
                        base,
                    };
                    index.files.insert(
                        path.clone(),
                        Indexed {
                            stat,
                            content,
                            version: Some(version.id),
                        },
                    );
                    versions.push(version);
                    saved.push(path);
                }
                Seen::Gone { path } => {
                    let Some(known) = index.files.remove(&path) else {
                        continue;
                    };
                    let Some(v) = known.version else { continue };
                    // Only a delete of what is still the head; if a peer
                    // changed it meanwhile, the apply brings it back.
                    if tree.heads(&path).iter().any(|h| h.id == v) {
                        versions.push(FileVersion {
                            id: VersionId(Uuid::new_v4()),
                            path,
                            content: None,
                            size: 0,
                            by: self.config.device.clone(),
                            at_us: now_us(),
                            base: vec![v],
                        });
                    }
                }
            }
        }
        manifest.push(&versions);
        Ok(saved)
    }

    /// Bring the working copy to the heads; how many files are still
    /// missing, and how many of those wait for room in the store.
    async fn apply(&self, manifest: &Manifest<'_>, dir: &Path, index: &mut Index) -> (u32, u32) {
        let tree = manifest.tree();
        let mut missing = 0u32;
        let mut deferred = 0u32;
        let mut closing = Vec::new();
        let paths: Vec<String> = tree.paths().map(str::to_owned).collect();
        for path in paths {
            let Some(settled) = tree.settled(&path) else {
                continue;
            };
            let target = match settled {
                Settled::One(v) => v,
                Settled::Fork { winner, losers } => {
                    for loser in &losers {
                        let copy = conflict_path(&path, &loser.by);
                        let at = dir.join(&copy);
                        if !at.exists()
                            && tree.current(&copy).is_none()
                            && let Some(content) = &loser.content
                            && let Err(e) = self.write_file(&at, content, loser.size).await
                        {
                            missing = missing.saturating_add(1);
                            if matches!(e, EngineError::NoRoom(_)) {
                                deferred = deferred.saturating_add(1);
                            }
                        }
                    }
                    let heads = tree.heads(&path);
                    closing.push(FileVersion {
                        id: VersionId(Uuid::new_v4()),
                        base: heads.iter().map(|h| h.id).collect(),
                        by: self.config.device.clone(),
                        ..winner.clone()
                    });
                    winner
                }
            };
            let file = dir.join(&path);
            let on_disk = std::fs::metadata(&file).ok().map(|m| Stat::of(&m));
            let known = index.files.get(&path).cloned();
            if known.as_ref().is_some_and(|k| k.version == Some(target.id)) {
                continue;
            }
            match (&known, on_disk) {
                // A change pending here: the scan makes it a fork.
                (Some(k), Some(stat)) if stat != k.stat => continue,
                // A file the index does not know: the next scan commits it.
                (None, Some(_)) => continue,
                _ => {}
            }
            let Some(content) = &target.content else {
                // A delete: only of bytes the line once held.
                if let Some(k) = known
                    && tree.ever_held(&path, &k.content)
                {
                    let _ = std::fs::remove_file(&file);
                    index.files.remove(&path);
                    if let Some(parent) = file.parent() {
                        remove_empty_up_to(parent, dir);
                    }
                }
                continue;
            };
            if let Some(k) = &known
                && &k.content == content
            {
                index.files.insert(
                    path,
                    Indexed {
                        version: Some(target.id),
                        ..k.clone()
                    },
                );
                continue;
            }
            // The bytes this file is replacing are kept if sync replaced
            // them; with no room for them, the file waits.
            if let Some(k) = &known
                && let Err(e) = self.keep_replaced(&file, k).await
            {
                tracing::debug!(%path, %e, "files: not replaced yet");
                missing = missing.saturating_add(1);
                if matches!(e, EngineError::NoRoom(_)) {
                    deferred = deferred.saturating_add(1);
                }
                continue;
            }
            match self.write_file(&file, content, target.size).await {
                Ok(stat) => {
                    index.files.insert(
                        path,
                        Indexed {
                            stat,
                            content: content.clone(),
                            version: Some(target.id),
                        },
                    );
                }
                Err(e) => {
                    tracing::debug!(%path, %e, "files: not fetched yet");
                    missing = missing.saturating_add(1);
                    if matches!(e, EngineError::NoRoom(_)) {
                        deferred = deferred.saturating_add(1);
                    }
                }
            }
        }
        manifest.push(&closing);
        (missing, deferred)
    }

    /// A working file about to be replaced is moved into the store (if
    /// not there already), so its version stays: within the store's
    /// budget, else the replacement waits ([`EngineError::NoRoom`]).
    async fn keep_replaced(&self, file: &Path, known: &Indexed) -> Result<(), EngineError> {
        let Ok(hash) = Sha256::from_hex(&known.content) else {
            return Ok(());
        };
        // Only the bytes the index knows: a file changed since is a fork.
        let unchanged = std::fs::metadata(file).is_ok_and(|m| Stat::of(&m) == known.stat);
        if !unchanged {
            return Ok(());
        }
        Ok(self.store.local.adopt(file, &hash).await?)
    }

    /// Write `content` at `at`: beside it first, then renamed into place,
    /// so a DAW sees the old file or the whole new one.
    async fn write_file(&self, at: &Path, content: &str, size: u64) -> Result<Stat, EngineError> {
        let hash = Sha256::from_hex(content).map_err(|e| EngineError::Io(e.to_string()))?;
        let source = if let Some(path) = self.store.file_for(&hash) {
            path
        } else {
            // Asked before any remote is: no room, no fetch (and no event
            // per pass for a file that keeps waiting).
            if !self.store.local.has_room(size) {
                return Err(EngineError::NoRoom(format!(
                    "no room in the store for {size} bytes"
                )));
            }
            self.fetcher.fetch(hash).await?;
            // The fetcher checked every byte against the hash.
            self.note_verified(content);
            self.store.local.path_for(&hash)
        };
        if let Some(parent) = at.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let part = PathBuf::from(format!("{}{PART}", at.display()));
        let from_store = source.starts_with(self.store.local.root());
        if from_store
            && size > self.config.keep_inline
            && !self.store.local.is_pinned(hash).await.unwrap_or(true)
        {
            // One copy on this device: it moves out of the store into the
            // working copy (copied across filesystems, then removed), and
            // is served from there.
            self.store.local.move_out(&hash, &part).await?;
        } else {
            tokio::fs::copy(&source, &part).await?;
        }
        tokio::fs::rename(&part, at).await?;
        let stat = Stat::of(&tokio::fs::metadata(at).await?);
        self.store.in_place.insert(hash, at.to_path_buf(), stat);
        Ok(stat)
    }

    /// Fetch a library's notes, so its assets read offline.
    async fn fetch_notes(&self, tree: &Tree) {
        for (path, v) in tree.live() {
            let small_note = std::path::Path::new(&path)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("md"))
                && v.size <= 1 << 20;
            if let (true, Some(content)) = (small_note, &v.content)
                && let Ok(hash) = Sha256::from_hex(content)
                && self.store.file_for(&hash).is_none()
            {
                if self.fetcher.fetch(hash).await.is_ok() {
                    self.note_verified(content);
                    self.attest(&[hash]).await;
                }
            }
        }
    }

    fn checkpoint(&self, manifest: Manifest<'_>, name: &str, auto: bool) -> Checkpoint {
        let heads = manifest
            .tree()
            .live()
            .into_iter()
            .map(|(p, v)| (p, v.id))
            .collect();
        let checkpoint = Checkpoint {
            id: VersionId(Uuid::new_v4()),
            name: name.to_owned(),
            by: self.config.device.clone(),
            at_us: now_us(),
            auto,
            heads,
        };
        manifest.push_checkpoint(&checkpoint);
        checkpoint
    }

    /// A checkpoint asked for by name.
    ///
    /// # Errors
    ///
    /// The root is not known.
    pub async fn make_checkpoint(
        &self,
        doc_id: Uuid,
        name: &str,
    ) -> Result<Checkpoint, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        Ok(self.checkpoint(Manifest::new(&doc), name, false))
    }

    /// The root's checkpoints, oldest first.
    ///
    /// # Errors
    ///
    /// The root is not known.
    pub async fn checkpoints(&self, doc_id: Uuid) -> Result<Vec<Checkpoint>, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        Ok(Manifest::new(&doc).checkpoints())
    }

    /// Put `path` back at `version`: a new version with its bytes, so
    /// nothing is lost.
    ///
    /// # Errors
    ///
    /// The root or the version is not known.
    pub async fn restore(
        &self,
        doc_id: Uuid,
        path: &str,
        version: VersionId,
    ) -> Result<FileVersion, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        let manifest = Manifest::new(&doc);
        let tree = manifest.tree();
        let old = tree
            .version(version)
            .filter(|v| v.path == path)
            .ok_or(EngineError::NoVersion)?;
        let restored = self.restoring(&tree, old.clone());
        manifest.push(std::slice::from_ref(&restored));
        Ok(restored)
    }

    /// Put the whole root back at `checkpoint`.
    ///
    /// # Errors
    ///
    /// The root or the checkpoint is not known.
    pub async fn restore_checkpoint(
        &self,
        doc_id: Uuid,
        checkpoint: VersionId,
    ) -> Result<u32, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        let manifest = Manifest::new(&doc);
        let cp = manifest
            .checkpoints()
            .into_iter()
            .find(|c| c.id == checkpoint)
            .ok_or(EngineError::NoVersion)?;
        let tree = manifest.tree();
        let live = tree.live();
        let mut versions = Vec::new();
        for (path, id) in &cp.heads {
            if live.get(path).is_some_and(|v| v.id == *id) {
                continue;
            }
            if let Some(old) = tree.version(*id) {
                versions.push(self.restoring(&tree, old.clone()));
            }
        }
        let wanted: BTreeSet<&String> = cp.heads.keys().collect();
        for (path, v) in &live {
            if !wanted.contains(path) {
                versions.push(FileVersion {
                    id: VersionId(Uuid::new_v4()),
                    path: path.clone(),
                    content: None,
                    size: 0,
                    by: self.config.device.clone(),
                    at_us: now_us(),
                    base: vec![v.id],
                });
            }
        }
        manifest.push(&versions);
        Ok(u32::try_from(versions.len()).unwrap_or(u32::MAX))
    }

    fn restoring(&self, tree: &Tree, old: FileVersion) -> FileVersion {
        FileVersion {
            id: VersionId(Uuid::new_v4()),
            base: tree.heads(&old.path).iter().map(|h| h.id).collect(),
            by: self.config.device.clone(),
            at_us: now_us(),
            ..old
        }
    }

    /// Save `bytes` at `path` in a root with no working copy here (a
    /// `PUT` over WebDAV): stored, then a version.
    ///
    /// # Errors
    ///
    /// The root is not known, or the store cannot be written.
    pub async fn save_bytes(
        &self,
        doc_id: Uuid,
        path: &str,
        bytes: &[u8],
    ) -> Result<FileVersion, EngineError> {
        self.save_reader(doc_id, path, Box::pin(std::io::Cursor::new(bytes.to_vec())))
            .await
    }

    /// [`Self::save_bytes`] from a stream (a `PUT`'s body), never all in memory.
    ///
    /// # Errors
    ///
    /// The root is not known, or the store cannot be written.
    pub async fn save_reader(
        &self,
        doc_id: Uuid,
        path: &str,
        body: architect::blob::Reader,
    ) -> Result<FileVersion, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        let manifest = Manifest::new(&doc);
        let hash = architect::blob::BlobStore::put(&self.store.local, None, body).await?;
        let size = tokio::fs::metadata(self.store.local.path_for(&hash))
            .await?
            .len();
        let version = FileVersion {
            id: VersionId(Uuid::new_v4()),
            path: path.to_owned(),
            content: Some(hash.to_hex()),
            size,
            by: self.config.device.clone(),
            at_us: now_us(),
            base: manifest.tree().heads(path).iter().map(|h| h.id).collect(),
        };
        manifest.push(std::slice::from_ref(&version));
        Ok(version)
    }

    /// Save `path` as `content` (bytes some version already names, in
    /// this root or another of the device's): a version and no bytes
    /// moved. An S3 `CopyObject` across roots.
    ///
    /// # Errors
    ///
    /// The root is not known.
    pub async fn save_content(
        &self,
        doc_id: Uuid,
        path: &str,
        content: Sha256,
        size: u64,
    ) -> Result<FileVersion, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        let manifest = Manifest::new(&doc);
        let version = FileVersion {
            id: VersionId(Uuid::new_v4()),
            path: path.to_owned(),
            content: Some(content.to_hex()),
            size,
            by: self.config.device.clone(),
            at_us: now_us(),
            base: manifest.tree().heads(path).iter().map(|h| h.id).collect(),
        };
        manifest.push(std::slice::from_ref(&version));
        Ok(version)
    }

    /// Move or copy `from` (a file, or everything under it) to `to`: the
    /// same bytes at the new paths, so nothing is sent again.
    ///
    /// # Errors
    ///
    /// The root is not known.
    pub async fn relocate(
        &self,
        doc_id: Uuid,
        from: &str,
        to: &str,
        remove: bool,
    ) -> Result<u32, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        let manifest = Manifest::new(&doc);
        let tree = manifest.tree();
        let prefix = format!("{from}/");
        let mut versions = Vec::new();
        for (path, v) in tree.live() {
            let rest = if path == from {
                ""
            } else if let Some(rest) = path.strip_prefix(&prefix) {
                rest
            } else {
                continue;
            };
            let target = if rest.is_empty() {
                to.to_owned()
            } else {
                format!("{to}/{rest}")
            };
            versions.push(FileVersion {
                id: VersionId(Uuid::new_v4()),
                path: target.clone(),
                by: self.config.device.clone(),
                at_us: now_us(),
                base: tree.heads(&target).iter().map(|h| h.id).collect(),
                ..v.clone()
            });
            if remove {
                versions.push(FileVersion {
                    id: VersionId(Uuid::new_v4()),
                    path,
                    content: None,
                    size: 0,
                    by: self.config.device.clone(),
                    at_us: now_us(),
                    base: vec![v.id],
                });
            }
        }
        manifest.push(&versions);
        Ok(u32::try_from(versions.len()).unwrap_or(u32::MAX))
    }

    /// Delete `path` (or everything under it) in a root, as versions.
    ///
    /// # Errors
    ///
    /// The root is not known.
    pub async fn delete_path(&self, doc_id: Uuid, path: &str) -> Result<u32, EngineError> {
        let doc = self
            .doc(doc_id)
            .await
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        let manifest = Manifest::new(&doc);
        let prefix = format!("{path}/");
        let versions: Vec<FileVersion> = manifest
            .tree()
            .live()
            .into_iter()
            .filter(|(p, _)| p == path || p.starts_with(&prefix))
            .map(|(p, v)| FileVersion {
                id: VersionId(Uuid::new_v4()),
                path: p,
                content: None,
                size: 0,
                by: self.config.device.clone(),
                at_us: now_us(),
                base: vec![v.id],
            })
            .collect();
        manifest.push(&versions);
        Ok(u32::try_from(versions.len()).unwrap_or(u32::MAX))
    }

    /// Every root's status.
    pub async fn status(&self) -> Vec<RootStatus> {
        let mut out = Vec::new();
        for doc_id in self.known() {
            if let Some(s) = self.root_status(doc_id).await {
                out.push(s);
            }
        }
        out.sort_by(|a, b| a.place.cmp(&b.place));
        out
    }

    /// One root's status.
    pub async fn root_status(&self, doc_id: Uuid) -> Option<RootStatus> {
        let doc = self.doc(doc_id).await?;
        let manifest = Manifest::new(&doc);
        let meta = manifest.meta().or_else(|| self.meta(doc_id))?;
        let live = manifest.tree().live();
        let dir = self.working_dir(doc_id);
        let pinned = self.pins().roots.contains_key(&doc_id.to_string());
        let at_head = |index: &Index| {
            live.iter()
                .all(|(p, v)| index.files.get(p).is_some_and(|i| i.version == Some(v.id)))
        };
        let seen = self.lock().get(&doc_id).map(|s| {
            (
                s.index.as_ref().is_some_and(at_head),
                s.missing,
                s.deferred,
                s.scanner.stubs.clone(),
                s.error.clone(),
            )
        });
        let (at_heads, missing, deferred, stubs, error) = seen.unwrap_or_default();
        let synced = dir.is_some() && at_heads;
        Some(RootStatus {
            doc_id: doc_id.to_string(),
            place: meta.place(),
            files: u32::try_from(live.len()).unwrap_or(u32::MAX),
            bytes: live.values().map(|v| v.size).sum(),
            org: meta.org,
            section: meta.section,
            name: meta.name,
            path: dir.map(|d| d.display().to_string()),
            pinned,
            missing,
            deferred,
            synced,
            stubs,
            error,
        })
    }

    /// Whether this device holds `content` (any root, any version).
    #[must_use]
    pub fn holds(&self, content: &Sha256) -> bool {
        self.store.file_for(content).is_some()
    }
}

/// Now, µs since the epoch.
#[must_use]
pub fn now_us() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_micros()).ok())
        .unwrap_or(0)
}

fn list_files(dir: &Path, prefix: &str, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if layout::ignored(&rel) {
            continue;
        }
        match entry.file_type() {
            Ok(t) if t.is_dir() => list_files(&entry.path(), &rel, out),
            Ok(t) if t.is_file() => out.push(rel),
            _ => {}
        }
    }
}

fn remove_empty_dirs(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                remove_empty_dirs(&entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir(dir);
}

fn remove_empty_up_to(from: &Path, root: &Path) {
    let mut at = from.to_path_buf();
    while at.starts_with(root) && at != root {
        if std::fs::remove_dir(&at).is_err() {
            break;
        }
        let Some(parent) = at.parent() else { break };
        at = parent.to_path_buf();
    }
}
