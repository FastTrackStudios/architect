//! The engine's copies: the registry kept current, this device's
//! attestations kept true, scrubs, challenges, and the reports and policy
//! over every holder's ledger. The spec:
//! `docs/content/architecture/file-registry.md`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use architect::blob::Sha256;
use loro::{Frontiers, LoroDoc, LoroValue};
use uuid::Uuid;

use super::engine::{Engine, EngineError, now_us};
use super::ledger::{self, HolderLedger, Identity, LedgerDoc, Writer};
use super::manifest::Manifest;
use super::registry::{Found, Query, Registry, RootIndex, Row};
use super::scan::Index;
use super::verify::{self, PossessionError, Proof, Prover};
use crate::copies::{
    Attestation, Check, Confidence, Copy, CopyReport, CopyState, Holder, HolderKind, HolderShare,
    HolderStatus, Locations, Place, Policy, ScrubReport, Verdict, Violation, confidence,
};
use crate::roots::{Hit, Section};

/// How many files a report lists under policy.
const VIOLATIONS_SHOWN: usize = 500;

/// One ledger document, as last read.
struct DocCache {
    which: LedgerDoc,
    frontiers: Frontiers,
    ledger: HolderLedger,
    policies: BTreeMap<String, Policy>,
}

/// Every ledger this device holds, read and checked.
#[derive(Default)]
struct LedgerCache {
    docs: HashMap<Uuid, DocCache>,
    /// org → (hash → `(verifier, check)`), every verifier's checks merged.
    checks: HashMap<String, HashMap<String, Vec<(String, Check)>>>,
    checks_dirty: bool,
}

impl LedgerCache {
    fn holders_in<'a>(&'a self, org: &'a str) -> impl Iterator<Item = &'a DocCache> + 'a {
        self.docs
            .values()
            .filter(move |d| matches!(&d.which, LedgerDoc::Holder { org: o, .. } if o == org))
    }

    fn merge_checks(&mut self) {
        if !self.checks_dirty {
            return;
        }
        self.checks_dirty = false;
        self.checks.clear();
        for d in self.docs.values() {
            let LedgerDoc::Holder { org, holder } = &d.which else {
                continue;
            };
            for c in d.ledger.checks.values() {
                self.checks
                    .entry(org.clone())
                    .or_default()
                    .entry(c.hash.clone())
                    .or_default()
                    .push((holder.clone(), c.clone()));
            }
        }
    }

    fn policy(&self, org: &str, place: &str) -> Policy {
        let docs = self
            .docs
            .values()
            .filter(|d| matches!(&d.which, LedgerDoc::Policy { org: o } if o == org));
        let mut root = None;
        let mut whole = None;
        for d in docs {
            root = root.or_else(|| d.policies.get(place).copied());
            whole = whole.or_else(|| d.policies.get("").copied());
        }
        root.or(whole).unwrap_or_default()
    }
}

/// What the engine keeps for its copies. Cheap to share.
pub(super) struct Books {
    identity: RwLock<Identity>,
    registry: Mutex<Registry>,
    cache: Mutex<LedgerCache>,
    /// The ledger documents this device knows.
    ledgers: Mutex<BTreeMap<Uuid, LedgerDoc>>,
    /// Holders reachable now, and how to challenge them.
    provers: RwLock<BTreeMap<String, Arc<dyn Prover>>>,
    /// Hashes whose bytes were just verified on arrival (fetched,
    /// committed), awaiting their attestation's `verified_at`.
    fresh: Mutex<HashMap<String, i64>>,
    /// Per root, the hashes its working copy holds as last attested.
    by_root: Mutex<HashMap<Uuid, HashSet<String>>>,
    /// Per root this device does not hold, the peers that list it.
    listings: Mutex<HashMap<Uuid, BTreeSet<String>>>,
    /// The S3 holders this device manages.
    managed: RwLock<BTreeMap<String, Holder>>,
    /// Hashes a scrub found bad here: not attested again until fresh
    /// bytes arrive (a fetch, a commit), however the working copy looks.
    damaged: Mutex<HashSet<String>>,
    /// Where the ledger documents this device keeps are listed, so a
    /// restart lists (and reads) them again before writing any.
    ledgers_file: PathBuf,
}

/// The kept ledger names, one per line.
fn load_ledgers(file: &std::path::Path) -> BTreeMap<Uuid, LedgerDoc> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(LedgerDoc::parse)
        .map(|l| (l.doc_id(), l))
        .collect()
}

fn save_ledgers(file: &std::path::Path, ledgers: &BTreeMap<Uuid, LedgerDoc>) {
    let text = ledgers.values().fold(String::new(), |mut text, l| {
        text.push_str(&l.name());
        text.push('\n');
        text
    });
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(file, text);
}

impl Books {
    pub(super) fn new(data: &std::path::Path) -> Self {
        let identity = Identity::load_or_create(&data.join("holder.key"))
            .unwrap_or_else(|_| Identity::from_secret(rand::random()));
        Self {
            identity: RwLock::new(identity),
            registry: Mutex::new(Registry::new(data.join("registry"))),
            cache: Mutex::default(),
            provers: RwLock::default(),
            fresh: Mutex::default(),
            by_root: Mutex::default(),
            listings: Mutex::default(),
            managed: RwLock::default(),
            damaged: Mutex::default(),
            ledgers: Mutex::new(load_ledgers(&data.join("ledgers.txt"))),
            ledgers_file: data.join("ledgers.txt"),
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read<T>(m: &RwLock<T>) -> std::sync::RwLockReadGuard<'_, T> {
    m.read().unwrap_or_else(PoisonError::into_inner)
}

fn write<T>(m: &RwLock<T>) -> std::sync::RwLockWriteGuard<'_, T> {
    m.write().unwrap_or_else(PoisonError::into_inner)
}

/// What this device holds of one blob.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Holding {
    state: CopyState,
    size: u64,
    have: u64,
}

/// One holder's view of a blob, merged over the orgs that name it.
struct Seen {
    holder: Holder,
    att: Attestation,
    verdict: Verdict,
}

impl Engine {
    // ── Identity and holders ───────────────────────────────────────

    /// Sign with `secret` (a node's iroh key, so the holder id is its
    /// endpoint id) instead of the key kept in the data dir.
    #[must_use]
    pub fn with_identity(self, secret: [u8; 32]) -> Self {
        *write(&self.books.identity) = Identity::from_secret(secret);
        self
    }

    /// This device's holder id.
    #[must_use]
    pub fn holder_id(&self) -> String {
        read(&self.books.identity).id().to_owned()
    }

    /// This device as a holder.
    #[must_use]
    pub fn holder(&self) -> Holder {
        let c = self.config();
        Holder {
            id: self.holder_id(),
            kind: if c.hold_all {
                HolderKind::Node
            } else {
                HolderKind::Device
            },
            name: c.device.clone(),
            site: if c.site.is_empty() {
                c.device.clone()
            } else {
                c.site.clone()
            },
            manager: None,
        }
    }

    /// The holders reachable now and how to challenge them (the node's
    /// peer connections; an S3 bucket's prover on its manager).
    pub fn set_provers(&self, provers: Vec<(String, Arc<dyn Prover>)>) {
        *write(&self.books.provers) = provers.into_iter().collect();
    }

    /// One more reachable holder.
    pub fn add_prover(&self, holder: impl Into<String>, prover: Arc<dyn Prover>) {
        write(&self.books.provers).insert(holder.into(), prover);
    }

    /// A holder is no longer reachable.
    pub fn remove_prover(&self, holder: &str) {
        write(&self.books.provers).remove(holder);
    }

    /// Whether `holder` is this device or one reachable now.
    #[must_use]
    pub fn reachable(&self, holder: &str) -> bool {
        holder == self.holder_id() || read(&self.books.provers).contains_key(holder)
    }

    // ── Ledger documents on the lane ───────────────────────────────

    /// A ledger document another device named on the lane: kept from now on.
    #[allow(clippy::must_use_candidate)]
    pub fn learn_ledger(&self, which: LedgerDoc) -> Uuid {
        let id = which.doc_id();
        let mut ledgers = lock(&self.books.ledgers);
        if ledgers.insert(id, which).is_none() {
            save_ledgers(&self.books.ledgers_file, &ledgers);
        }
        drop(ledgers);
        id
    }

    /// Whether `doc_id` is a ledger document this device keeps.
    #[must_use]
    pub fn knows_ledger(&self, doc_id: Uuid) -> bool {
        lock(&self.books.ledgers).contains_key(&doc_id)
    }

    /// The ledger documents this device keeps, by id.
    #[must_use]
    pub fn ledgers(&self) -> Vec<(Uuid, LedgerDoc)> {
        lock(&self.books.ledgers)
            .iter()
            .map(|(id, l)| (*id, l.clone()))
            .collect()
    }

    /// `hash`'s bytes were just checked against it on arrival (a fetch,
    /// a commit): its next attestation is verified now.
    pub fn note_verified(&self, hash: &str) {
        lock(&self.books.fresh).insert(hash.to_owned(), now_us());
        lock(&self.books.damaged).remove(hash);
    }

    // ── The registry ───────────────────────────────────────────────

    /// Re-read every root whose manifest moved since the registry last saw it.
    pub async fn refresh_registry(&self) {
        lock(&self.books.registry).load();
        for doc_id in self.known() {
            let Some(doc) = self.doc(doc_id).await else {
                continue;
            };
            let digest = super::lane::digest_of(&doc);
            if lock(&self.books.registry).digest(doc_id) == Some(digest.as_str()) {
                continue;
            }
            let Some(meta) = Manifest::new(&doc).meta() else {
                continue;
            };
            let index = self.index_root(doc_id, &doc, &meta, digest).await;
            lock(&self.books.registry).put(index);
        }
    }

    async fn index_root(
        &self,
        doc_id: Uuid,
        doc: &LoroDoc,
        meta: &super::manifest::RootMeta,
        digest: String,
    ) -> RootIndex {
        let manifest = Manifest::new(doc);
        let versions = manifest.versions();
        let hashes: BTreeSet<String> = versions.iter().filter_map(|v| v.content.clone()).collect();
        let tree = super::manifest::Tree::new(versions);
        let mut rows = vec![Row {
            path: String::new(),
            size: tree.used(),
            at_us: 0,
            content: String::new(),
            versions: 0,
            what: match meta.section {
                Section::Projects => "project".to_owned(),
                Section::Assets => "library".to_owned(),
            },
            props: String::new(),
            name: String::new(),
        }];
        // path → (searched text, what, the asset's name)
        let mut props: HashMap<String, (String, String, String)> = HashMap::new();
        if meta.section == Section::Assets
            && let Ok(assets) = self.assets(doc_id).await
        {
            for a in assets {
                let values: Vec<String> = a.properties.values().cloned().collect();
                let what = a
                    .properties
                    .get("type")
                    .cloned()
                    .unwrap_or_else(|| "asset".to_owned());
                let joined = format!("{} {}", a.name, values.join(" "));
                for m in &a.media {
                    props.insert(
                        m.path.clone(),
                        (joined.clone(), String::new(), String::new()),
                    );
                }
                if let Some(note) = &a.note {
                    props.insert(note.clone(), (joined.clone(), what, a.name.clone()));
                }
            }
        }
        for (path, v) in tree.live() {
            let (p, what, name) = props.remove(&path).unwrap_or_default();
            rows.push(Row {
                versions: u32::try_from(tree.history(&path).len()).unwrap_or(u32::MAX),
                size: v.size,
                at_us: v.at_us,
                content: v.content.clone().unwrap_or_default(),
                what: if what.is_empty() {
                    "file".to_owned()
                } else {
                    what
                },
                props: p,
                name,
                path,
            });
        }
        RootIndex {
            doc_id: doc_id.to_string(),
            org: meta.org.clone(),
            place: meta.place(),
            name: meta.name.clone(),
            section: meta.section.clone(),
            digest,
            rows,
            hashes: hashes.into_iter().collect(),
        }
    }

    /// The registry's size: roots and rows.
    #[must_use]
    pub fn registry_size(&self) -> (usize, usize) {
        let mut r = lock(&self.books.registry);
        r.load();
        (r.roots().count(), r.len())
    }

    /// Put a root's index straight in (a benchmark, an import).
    pub fn put_index(&self, index: RootIndex) {
        let mut r = lock(&self.books.registry);
        r.load();
        r.put(index);
    }

    /// Search the registry, offline; each file hit says where its bytes are.
    pub async fn registry_search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let q = Query::parse(query);
        let found = {
            let mut r = lock(&self.books.registry);
            r.load();
            r.search(&q, limit)
        };
        self.refresh_ledgers_if_empty().await;
        found.into_iter().map(|f| self.hit(f)).collect()
    }

    fn hit(&self, f: Found) -> Hit {
        let Found {
            org,
            place,
            root_name,
            row,
            ..
        } = f;
        let name = if row.path.is_empty() {
            root_name
        } else if !row.name.is_empty() {
            row.name.clone()
        } else {
            row.path.rsplit('/').next().unwrap_or(&row.path).to_owned()
        };
        let mut hit = Hit {
            root: place,
            path: row.path.clone(),
            name,
            what: row.what.clone(),
            detail: if row.props.is_empty() {
                format!("{} bytes, {} versions", row.size, row.versions)
            } else {
                row.props.clone()
            },
            org,
            size: row.size,
            content: (!row.content.is_empty()).then(|| row.content.clone()),
            holders: Vec::new(),
            verified: 0,
            reachable: false,
        };
        if !row.content.is_empty() {
            let seen = self.seen(&row.content, Some(row.size));
            let mut seen: Vec<&Seen> = seen.iter().collect();
            seen.sort_by_key(|s| s.verdict.confidence);
            hit.verified = u32::try_from(
                seen.iter()
                    .filter(|s| s.verdict.confidence == Confidence::Verified)
                    .count(),
            )
            .unwrap_or(u32::MAX);
            hit.reachable = seen
                .iter()
                .any(|s| s.att.state.counts() && self.reachable(&s.holder.id))
                || Sha256::from_hex(&row.content).is_ok_and(|h| self.holds(&h));
            hit.holders = seen
                .iter()
                .map(|s| format!("{} ({})", s.holder.name, s.verdict.confidence.label()))
                .collect();
        }
        hit
    }

    /// A revoked or vanished root: its rows, its state, its listing.
    pub fn forget(&self, doc_id: Uuid) {
        lock(&self.books.registry).remove(doc_id);
        lock(&self.books.listings).remove(&doc_id);
        self.forget_root(doc_id);
    }

    /// `peer` listed `docs` (the roots and ledgers it lets this device
    /// open). A root or ledger it listed before and no longer does, that
    /// no other peer lists and that this device does not hold, is
    /// forgotten: access was revoked. The ids forgotten.
    #[allow(clippy::must_use_candidate)]
    pub fn listed_by(&self, peer: &str, docs: &[Uuid]) -> Vec<Uuid> {
        let now: HashSet<Uuid> = docs.iter().copied().collect();
        let mut forgotten = Vec::new();
        {
            let mut listings = lock(&self.books.listings);
            for id in &now {
                listings.entry(*id).or_default().insert(peer.to_owned());
            }
            for (id, peers) in listings.iter_mut() {
                if !now.contains(id) && peers.remove(peer) && peers.is_empty() {
                    forgotten.push(*id);
                }
            }
            for id in &forgotten {
                listings.remove(id);
            }
        }
        forgotten.retain(|id| self.working_dir(*id).is_none());
        for id in &forgotten {
            let removed = {
                let mut ledgers = lock(&self.books.ledgers);
                let removed = ledgers.remove(id).is_some();
                if removed {
                    save_ledgers(&self.books.ledgers_file, &ledgers);
                }
                drop(ledgers);
                removed
            };
            if removed {
                let mut cache = lock(&self.books.cache);
                cache.docs.remove(id);
                cache.checks_dirty = true;
            } else {
                self.forget(*id);
            }
        }
        forgotten
    }

    // ── Reading every ledger ───────────────────────────────────────

    /// Re-read the ledger documents that moved: only the entries that
    /// changed, each signature checked once.
    pub async fn refresh_ledgers(&self) {
        for (id, which) in self.ledgers() {
            let Some(doc) = self.doc(id).await else {
                continue;
            };
            let frontiers = doc.state_frontiers();
            let old = {
                let cache = lock(&self.books.cache);
                cache.docs.get(&id).map(|d| d.frontiers.clone())
            };
            if old.as_ref() == Some(&frontiers) {
                continue;
            }
            let changed = old.and_then(|o| changed_keys(&doc, &o, &frontiers));
            let entry = lock(&self.books.cache).docs.remove(&id);
            let read = tokio::task::spawn_blocking(move || {
                read_doc(&doc, which, frontiers, entry, changed)
            })
            .await;
            let mut cache = lock(&self.books.cache);
            if let Ok(updated) = read {
                cache.docs.insert(id, updated);
            }
            cache.checks_dirty = true;
        }
    }

    async fn refresh_ledgers_if_empty(&self) {
        let empty = lock(&self.books.cache).docs.is_empty();
        if empty {
            self.refresh_ledgers().await;
        }
    }

    /// Every holder's view of `hash` this device can see, merged per holder.
    fn seen(&self, hash: &str, size: Option<u64>) -> Vec<Seen> {
        let orgs = lock(&self.books.registry).orgs_of(hash);
        let window = i64::try_from(self.config().verify_window.as_micros()).unwrap_or(i64::MAX);
        let now = now_us();
        let mut cache = lock(&self.books.cache);
        cache.merge_checks();
        let mut best: BTreeMap<String, (Holder, Attestation)> = BTreeMap::new();
        let mut checks: Vec<(String, Check)> = Vec::new();
        for org in &orgs {
            for d in cache.holders_in(org) {
                let (Some(att), LedgerDoc::Holder { holder, .. }) =
                    (d.ledger.copies.get(hash), &d.which)
                else {
                    continue;
                };
                let record = d.ledger.holder.clone().unwrap_or_else(|| Holder {
                    id: holder.clone(),
                    kind: HolderKind::Device,
                    name: holder.chars().take(8).collect(),
                    site: String::new(),
                    manager: None,
                });
                let newer = best.get(holder).is_none_or(|(_, a)| a.at_us < att.at_us);
                if newer {
                    best.insert(holder.clone(), (record, att.clone()));
                }
            }
            if let Some(cs) = cache.checks.get(org).and_then(|m| m.get(hash)) {
                checks.extend(cs.iter().cloned());
            }
        }
        // A verifier is shown by its name, not its key.
        let names: HashMap<String, String> = cache
            .docs
            .values()
            .filter_map(|d| d.ledger.holder.as_ref())
            .map(|h| (h.id.clone(), h.name.clone()))
            .collect();
        drop(cache);
        let me = self.holder_id();
        let name_of = |id: &str| -> String {
            if id == me {
                self.config().device.clone()
            } else {
                names
                    .get(id)
                    .map_or_else(|| id.chars().take(8).collect(), Clone::clone)
            }
        };
        best.into_iter()
            .map(|(id, (holder, att))| {
                let by_name: Vec<(String, &Check)> = checks
                    .iter()
                    .filter(|(by, c)| c.holder == id && by != &id)
                    .map(|(by, c)| (name_of(by), c))
                    .collect();
                let of_it = by_name.iter().map(|(by, c)| (by.as_str(), *c));
                let verdict = confidence(&att, size, of_it, now, window);
                Seen {
                    holder,
                    att,
                    verdict,
                }
            })
            .collect()
    }

    // ── Holdings and attestations ──────────────────────────────────

    /// What this device holds of `hash`, looked at now.
    fn holding_of(&self, hash: &Sha256) -> Option<Holding> {
        let stored = self
            .store
            .stored_for(hash)
            .unwrap_or_else(|| self.store.local.path_for(hash));
        if let Some(path) = self.store.in_place.find(hash) {
            let size = std::fs::metadata(path).ok()?.len();
            return Some(Holding {
                state: CopyState::Full,
                size,
                have: size,
            });
        }
        if let Ok(m) = std::fs::metadata(&stored)
            && m.is_file()
        {
            return Some(Holding {
                state: if self.config().hold_all {
                    CopyState::Full
                } else {
                    CopyState::Cached
                },
                size: m.len(),
                have: m.len(),
            });
        }
        let part = architect::blob::part_path(&stored);
        let m = std::fs::metadata(part).ok()?;
        let size = self
            .fetcher
            .progress(*hash)
            .and_then(|p| match &*p.borrow() {
                architect::blob::Progress::Fetching { size, .. } => Some(*size),
                _ => None,
            })
            .unwrap_or(0);
        Some(Holding {
            state: CopyState::Partial,
            size: size.max(m.len()),
            have: m.len(),
        })
    }

    /// The blobs in the store (and the downloads growing there).
    fn stored(&self) -> HashMap<String, Holding> {
        let mut out = HashMap::new();
        let roots: Vec<PathBuf> = self
            .store
            .areas()
            .into_iter()
            .map(|a| a.root().to_owned())
            .collect();
        let state = if self.config().hold_all {
            CopyState::Full
        } else {
            CopyState::Cached
        };
        for dir in roots
            .iter()
            .filter_map(|r| std::fs::read_dir(r).ok())
            .flat_map(Iterator::flatten)
        {
            if dir.file_name().len() != 2 {
                continue;
            }
            let Ok(files) = std::fs::read_dir(dir.path()) else {
                continue;
            };
            for f in files.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                let Ok(meta) = f.metadata() else { continue };
                if name.len() == 64 {
                    out.insert(
                        name,
                        Holding {
                            state,
                            size: meta.len(),
                            have: meta.len(),
                        },
                    );
                } else if let Some(hex) = name.strip_suffix(".part").filter(|h| h.len() == 64)
                    && let Ok(hash) = Sha256::from_hex(hex)
                    && let Some(h) = self.holding_of(&hash)
                {
                    out.entry(hex.to_owned()).or_insert(h);
                }
            }
        }
        out
    }

    /// This device's own attestations in `org`, as last read.
    fn own_in(&self, org: &str) -> BTreeMap<String, Attestation> {
        let id = LedgerDoc::Holder {
            org: org.to_owned(),
            holder: self.holder_id(),
        }
        .doc_id();
        lock(&self.books.cache)
            .docs
            .get(&id)
            .map(|d| d.ledger.copies.clone())
            .unwrap_or_default()
    }

    /// Write `wanted` (hash → holding, `None` withdraws) into this
    /// device's ledger for `org`, only what differs.
    async fn write_own(&self, org: &str, wanted: &BTreeMap<String, Option<Holding>>) {
        if wanted.is_empty() {
            return;
        }
        let me = self.holder();
        let which = LedgerDoc::Holder {
            org: org.to_owned(),
            holder: me.id.clone(),
        };
        let id = self.learn_ledger(which.clone());
        let Some(doc) = self.doc(id).await else {
            return;
        };
        let current = ledger::read(&doc).copies;
        let fresh = lock(&self.books.fresh).clone();
        let identity = read(&self.books.identity).clone();
        let mut w = Writer::new(&doc, org, &identity, &me.id);
        w.holder(&me);
        for (hash, want) in wanted {
            let old = current.get(hash);
            match want {
                None => w.remove(hash),
                Some(h) => {
                    let same_bytes = old.is_some_and(|o| o.size == h.size);
                    let verified = fresh
                        .get(hash)
                        .copied()
                        .into_iter()
                        .chain(old.filter(|_| same_bytes).map(|o| o.verified_at_us))
                        .max()
                        .unwrap_or(0);
                    let att = Attestation {
                        hash: hash.clone(),
                        size: h.size,
                        state: h.state,
                        verified_at_us: verified,
                        merkle: old
                            .filter(|_| same_bytes)
                            .map(|o| o.merkle.clone())
                            .unwrap_or_default(),
                        have: h.have,
                        at_us: now_us(),
                        sig: String::new(),
                    };
                    let unchanged = old.is_some_and(|o| {
                        o.state == att.state
                            && o.size == att.size
                            && o.have == att.have
                            && o.verified_at_us == att.verified_at_us
                    });
                    if !unchanged {
                        w.put(att);
                    }
                }
            }
        }
        w.commit();
        let mut f = lock(&self.books.fresh);
        for hash in wanted.keys() {
            f.remove(hash);
        }
    }

    /// After a pass over a root: its working copy's bytes attested
    /// `full`, and what it no longer holds re-looked at (still cached,
    /// or withdrawn). Pins, pulls, commits and deletes land here.
    pub(super) async fn attest_root(&self, doc_id: Uuid, org: &str, index: &Index) {
        // After a restart, read what this device already attests first.
        self.refresh_ledgers_if_empty().await;
        let damaged = lock(&self.books.damaged).clone();
        let now: HashSet<String> = index
            .files
            .values()
            .map(|i| i.content.clone())
            .filter(|c| !damaged.contains(c))
            .collect();
        let before = lock(&self.books.by_root).get(&doc_id).cloned();
        let own = self.own_in(org);
        let mut wanted = BTreeMap::new();
        for i in index.files.values().filter(|i| now.contains(&i.content)) {
            let h = Holding {
                state: CopyState::Full,
                size: i.stat.size,
                have: i.stat.size,
            };
            let fresh = lock(&self.books.fresh).contains_key(&i.content);
            let same = own
                .get(&i.content)
                .is_some_and(|a| a.state == CopyState::Full && a.size == h.size);
            if !same || fresh {
                wanted.insert(i.content.clone(), Some(h));
            }
        }
        for gone in before.iter().flatten().filter(|h| !now.contains(*h)) {
            let held = Sha256::from_hex(gone)
                .ok()
                .and_then(|h| self.holding_of(&h));
            wanted.insert(gone.clone(), held);
        }
        self.write_own(org, &wanted).await;
        lock(&self.books.by_root).insert(doc_id, now);
    }

    /// Re-look at `hashes` now and attest what this device holds of
    /// them, in every org that names them (an evict, an unpin, a fetch).
    pub async fn attest(&self, hashes: &[Sha256]) {
        let mut by_org: BTreeMap<String, BTreeMap<String, Option<Holding>>> = BTreeMap::new();
        for hash in hashes {
            let hex = hash.to_hex();
            let held = self
                .holding_of(hash)
                .filter(|_| !lock(&self.books.damaged).contains(&hex));
            let orgs = lock(&self.books.registry).orgs_of(&hex);
            for org in orgs {
                by_org.entry(org).or_default().insert(hex.clone(), held);
            }
        }
        for (org, wanted) in by_org {
            self.write_own(&org, &wanted).await;
        }
        self.refresh_ledgers().await;
    }

    /// Bring this device's attestations to what it holds: the store's
    /// blobs and downloads, the working copies' bytes; withdraw the rest.
    pub async fn reconcile(&self) {
        let stored = self.stored();
        let in_roots: HashSet<String> = lock(&self.books.by_root)
            .values()
            .flatten()
            .cloned()
            .collect();
        let all = lock(&self.books.registry).all_hashes();
        let mut by_org: BTreeMap<String, BTreeMap<String, Option<Holding>>> = BTreeMap::new();
        let mut owns: BTreeMap<String, BTreeMap<String, Attestation>> = BTreeMap::new();
        for (hash, orgs) in all {
            let Some(h) = stored.get(&hash) else { continue };
            if in_roots.contains(&hash) {
                continue;
            }
            for org in orgs {
                let own = owns.entry(org.clone()).or_insert_with(|| self.own_in(&org));
                let same = own
                    .get(&hash)
                    .is_some_and(|a| a.state == h.state && a.have == h.have && a.size == h.size);
                if !same {
                    by_org
                        .entry(org)
                        .or_default()
                        .insert(hash.clone(), Some(*h));
                }
            }
        }
        // What is attested and no longer here at all.
        let orgs: BTreeSet<String> = self
            .ledgers()
            .into_iter()
            .filter_map(|(_, l)| match l {
                LedgerDoc::Holder { org, holder } if holder == self.holder_id() => Some(org),
                _ => None,
            })
            .collect();
        for org in orgs {
            let own = owns.remove(&org).unwrap_or_else(|| self.own_in(&org));
            for hash in own.keys() {
                if !stored.contains_key(hash) && !in_roots.contains(hash) {
                    by_org
                        .entry(org.clone())
                        .or_default()
                        .insert(hash.clone(), None);
                }
            }
        }
        for (org, wanted) in by_org {
            self.write_own(&org, &wanted).await;
        }
    }

    /// Attest blobs an S3 bucket this device manages holds: `holder` is
    /// the bucket (`kind: S3`, `manager` set to this device), signed by
    /// this device. `gone` withdraws.
    pub async fn attest_external(&self, holder: Holder, atts: Vec<Attestation>, gone: Vec<String>) {
        let mut holder = holder;
        holder.manager = Some(self.holder_id());
        write(&self.books.managed).insert(holder.id.clone(), holder.clone());
        let identity = read(&self.books.identity).clone();
        let mut by_org: BTreeMap<String, (Vec<Attestation>, Vec<String>)> = BTreeMap::new();
        for a in atts {
            let orgs = lock(&self.books.registry).orgs_of(&a.hash);
            for org in orgs {
                by_org.entry(org).or_default().0.push(a.clone());
            }
        }
        for g in gone {
            let orgs = lock(&self.books.registry).orgs_of(&g);
            for org in orgs {
                by_org.entry(org).or_default().1.push(g.clone());
            }
        }
        for (org, (puts, removes)) in by_org {
            let id = self.learn_ledger(LedgerDoc::Holder {
                org: org.clone(),
                holder: holder.id.clone(),
            });
            let Some(doc) = self.doc(id).await else {
                continue;
            };
            let mut w = Writer::new(&doc, &org, &identity, &holder.id);
            w.holder(&holder);
            for a in puts {
                w.put(Attestation {
                    at_us: now_us(),
                    ..a
                });
            }
            for g in removes {
                w.remove(&g);
            }
            w.commit();
        }
        self.refresh_ledgers().await;
    }

    /// A holder this device manages read `hash` back whole and found it
    /// right (an S3 bucket's scrub): its existing record, verified now.
    pub async fn restamp_external(&self, holder: &Holder, hash: &str) {
        let existing = lock(&self.books.cache)
            .docs
            .values()
            .find_map(|d| match &d.which {
                LedgerDoc::Holder { holder: h, .. } if h == &holder.id => {
                    d.ledger.copies.get(hash).cloned()
                }
                _ => None,
            });
        if let Some(att) = existing {
            let now = now_us();
            self.attest_external(
                holder.clone(),
                vec![Attestation {
                    verified_at_us: now,
                    ..att
                }],
                Vec::new(),
            )
            .await;
        }
    }

    // ── Scrubs ─────────────────────────────────────────────────────

    fn merkle_path(&self, hash: &str) -> PathBuf {
        self.config().data.join("merkle").join(hash)
    }

    /// Re-hash what this device holds: everything due (not verified within
    /// half the window), stalest first, up to `budget` bytes; with
    /// `scope` (an org, a root's place, a hash) everything in it, now.
    pub async fn scrub(&self, scope: Option<&str>, budget: u64) -> ScrubReport {
        self.refresh_ledgers().await;
        let me = self.holder_id();
        let half = i64::try_from(self.config().verify_window.as_micros() / 2).unwrap_or(i64::MAX);
        let now = now_us();
        // hash → (org, attestation) over this device's ledgers.
        let mut mine: BTreeMap<String, (Vec<String>, Attestation)> = BTreeMap::new();
        for (_, which) in self.ledgers() {
            let LedgerDoc::Holder { org, holder } = which else {
                continue;
            };
            if holder != me {
                continue;
            }
            for (hash, att) in self.own_in(&org) {
                let e = mine
                    .entry(hash)
                    .or_insert_with(|| (Vec::new(), att.clone()));
                e.0.push(org.clone());
            }
        }
        let in_scope = |hash: &str, orgs: &[String]| -> bool {
            let Some(s) = scope else { return true };
            if s == hash || orgs.iter().any(|o| o == s) {
                return true;
            }
            lock(&self.books.registry)
                .places(hash)
                .iter()
                .any(|p| p.place == s || p.place.starts_with(&format!("{s}/")))
        };
        let mut due: Vec<(String, Vec<String>, Attestation)> = mine
            .into_iter()
            .filter(|(_, (_, a))| a.state.counts())
            .filter(|(h, (orgs, a))| {
                if scope.is_some() {
                    in_scope(h, orgs)
                } else {
                    now.saturating_sub(a.verified_at_us) > half || a.merkle.is_empty()
                }
            })
            .map(|(h, (orgs, a))| (h, orgs, a))
            .collect();
        due.sort_by_key(|(_, _, a)| a.verified_at_us);
        let mut report = ScrubReport::default();
        for (hex, orgs, att) in due {
            if report.bytes >= budget && report.scrubbed > 0 {
                break;
            }
            let Ok(hash) = Sha256::from_hex(&hex) else {
                continue;
            };
            let file = self.store.file_for(&hash);
            let read = match file.clone() {
                Some(path) => tokio::task::spawn_blocking(move || verify::hash_file(&path))
                    .await
                    .ok()
                    .and_then(Result::ok),
                None => None,
            };
            report.scrubbed = report.scrubbed.saturating_add(1);
            report.bytes = report.bytes.saturating_add(att.size);
            match read {
                Some((sha, leaves)) if sha == hash => {
                    let root = super::scan::hex(&verify::root(&leaves));
                    let at = self.merkle_path(&hex);
                    if let Some(dir) = at.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(&at, verify::encode_leaves(&leaves));
                    self.restamp(
                        &orgs,
                        Attestation {
                            verified_at_us: now_us(),
                            merkle: root,
                            ..att
                        },
                    )
                    .await;
                }
                _ => {
                    report.corrupt.push(hex.clone());
                    lock(&self.books.damaged).insert(hex.clone());
                    tracing::warn!(hash = %hex, file = ?file, "files: a scrub found bad or missing bytes");
                    if let Some(path) =
                        file.filter(|p| self.store.areas().iter().any(|a| p.starts_with(a.root())))
                    {
                        self.quarantine_file(&path, &hex);
                    }
                    let _ = std::fs::remove_file(self.merkle_path(&hex));
                    let gone: BTreeMap<String, Option<Holding>> =
                        std::iter::once((hex.clone(), None)).collect();
                    for org in &orgs {
                        self.write_own(org, &gone).await;
                    }
                    if self.config().hold_all {
                        let fetcher = self.fetcher.clone();
                        tokio::spawn(async move {
                            let _ = fetcher.fetch(hash).await;
                        });
                    }
                }
            }
        }
        self.refresh_ledgers().await;
        report
    }

    /// Re-sign `att` (a fresh scrub) into this device's ledger for `orgs`.
    async fn restamp(&self, orgs: &[String], att: Attestation) {
        let me = self.holder();
        let identity = read(&self.books.identity).clone();
        for org in orgs {
            let id = self.learn_ledger(LedgerDoc::Holder {
                org: org.clone(),
                holder: me.id.clone(),
            });
            let Some(doc) = self.doc(id).await else {
                continue;
            };
            let mut w = Writer::new(&doc, org, &identity, &me.id);
            w.put(Attestation {
                at_us: now_us(),
                ..att.clone()
            });
            w.commit();
        }
    }

    // ── Challenges ─────────────────────────────────────────────────

    /// Prove this device holds chunk `index` of `hash`: the chunk and its path.
    ///
    /// # Errors
    ///
    /// Not held here, or not one any manifest here names.
    pub async fn prove(&self, hash: Sha256, index: u64) -> Result<Proof, PossessionError> {
        let served = read(&self.served).contains(&hash);
        let file = self
            .store
            .file_for(&hash)
            .filter(|_| served)
            .ok_or(PossessionError::NotHeld)?;
        let cache = self.merkle_path(&hash.to_hex());
        tokio::task::spawn_blocking(move || -> Result<Proof, PossessionError> {
            let size = std::fs::metadata(&file)
                .map_err(|e| PossessionError::Internal(e.to_string()))?
                .len();
            let mut leaves = std::fs::read(&cache)
                .map(|b| verify::decode_leaves(&b))
                .unwrap_or_default();
            if u64::try_from(leaves.len()).ok() != Some(verify::leaf_count(size)) {
                let (sha, l) = verify::hash_file(&file)
                    .map_err(|e| PossessionError::Internal(e.to_string()))?;
                if sha != hash {
                    return Err(PossessionError::NotHeld);
                }
                if let Some(dir) = cache.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(&cache, verify::encode_leaves(&l));
                leaves = l;
            }
            let chunk = verify::read_chunk(&file, index)
                .map_err(|e| PossessionError::Internal(e.to_string()))?;
            Ok(Proof {
                chunk,
                path: verify::path(&leaves, index)
                    .iter()
                    .map(|n| n.to_vec())
                    .collect(),
                leaves: u64::try_from(leaves.len()).unwrap_or(u64::MAX),
            })
        })
        .await
        .map_err(|e| PossessionError::Internal(e.to_string()))?
    }

    /// Challenge up to `max` copies held by reachable peers, stalest first;
    /// each result signed into this device's ledger.
    pub async fn challenge(&self, max: u32, scope_hash: Option<&str>) -> ScrubReport {
        self.refresh_ledgers().await;
        let me = self.holder_id();
        let provers = read(&self.books.provers).clone();
        let mut report = ScrubReport::default();
        if provers.is_empty() || max == 0 {
            return report;
        }
        // (evidence, holder, hash, size, orgs)
        let mut picks: Vec<(i64, String, String, u64)> = Vec::new();
        {
            let mut cache = lock(&self.books.cache);
            cache.merge_checks();
            for d in cache.docs.values() {
                let LedgerDoc::Holder { org, holder } = &d.which else {
                    continue;
                };
                if !provers.contains_key(holder) || holder == &me {
                    continue;
                }
                for (hash, att) in &d.ledger.copies {
                    if !att.state.counts() || scope_hash.is_some_and(|s| s != hash) {
                        continue;
                    }
                    let mine = cache
                        .checks
                        .get(org)
                        .and_then(|m| m.get(hash))
                        .into_iter()
                        .flatten()
                        .filter(|(by, c)| by == &me && &c.holder == holder)
                        .map(|(_, c)| c.at_us)
                        .max()
                        .unwrap_or(0);
                    picks.push((mine.max(0), holder.clone(), hash.clone(), att.size));
                }
            }
            drop(cache);
        }
        picks.sort();
        picks.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);
        for (_, holder, hash, size) in picks
            .into_iter()
            .take(usize::try_from(max).unwrap_or(usize::MAX))
        {
            let Some(prover) = provers.get(&holder) else {
                continue;
            };
            let Some((root, root_from)) = self.trusted_root(&hash, &holder) else {
                continue;
            };
            let Ok(sha) = Sha256::from_hex(&hash) else {
                continue;
            };
            let index = rand::random::<u64>()
                .checked_rem(verify::leaf_count(size))
                .unwrap_or(0);
            let answer =
                tokio::time::timeout(std::time::Duration::from_secs(30), prover.prove(sha, index))
                    .await;
            let (passed, why) = match answer {
                Ok(Ok(proof)) if proof.proves(&root, size, index) => (true, String::new()),
                Ok(Ok(_)) => (
                    false,
                    format!("chunk {index} does not prove under the root"),
                ),
                Ok(Err(verify::ProveError::Lacks(why))) => (false, why),
                // No answer is no evidence: the copy ages, it is not failed.
                Ok(Err(verify::ProveError::Unreachable(_))) | Err(_) => continue,
            };
            report.challenged = report.challenged.saturating_add(1);
            if passed {
                report.passed = report.passed.saturating_add(1);
            } else {
                report.failed.push(format!("{holder} {hash}: {why}"));
            }
            let check = Check {
                holder: holder.clone(),
                hash: hash.clone(),
                at_us: now_us(),
                passed,
                chunk: index,
                root_from,
                why,
                sig: String::new(),
            };
            self.write_check(&holder, check).await;
        }
        self.refresh_ledgers().await;
        report
    }

    /// The Merkle root to check `holder`'s proof of `hash` against: this
    /// device's, else another holder's, else the holder's own.
    fn trusted_root(&self, hash: &str, holder: &str) -> Option<(verify::Node, String)> {
        let me = self.holder_id();
        let roots: Vec<(String, verify::Node)> = lock(&self.books.cache)
            .docs
            .values()
            .filter_map(|d| {
                let LedgerDoc::Holder { holder: h, .. } = &d.which else {
                    return None;
                };
                let root = verify::node_of(&d.ledger.copies.get(hash)?.merkle)?;
                Some((h.clone(), root))
            })
            .collect();
        let mut theirs = None;
        let mut other = None;
        for (h, root) in roots {
            if h == me {
                return Some((root, me));
            } else if h == holder {
                theirs = Some((root, h));
            } else {
                other = Some((root, h));
            }
        }
        other.or(theirs)
    }

    async fn write_check(&self, holder: &str, check: Check) {
        let me = self.holder();
        let identity = read(&self.books.identity).clone();
        let orgs: Vec<String> = {
            let cache = lock(&self.books.cache);
            cache
                .docs
                .values()
                .filter_map(|d| match &d.which {
                    LedgerDoc::Holder { org, holder: h }
                        if h == holder && d.ledger.copies.contains_key(&check.hash) =>
                    {
                        Some(org.clone())
                    }
                    _ => None,
                })
                .collect()
        };
        for org in orgs {
            let id = self.learn_ledger(LedgerDoc::Holder {
                org: org.clone(),
                holder: me.id.clone(),
            });
            let Some(doc) = self.doc(id).await else {
                continue;
            };
            let mut w = Writer::new(&doc, &org, &identity, &me.id);
            w.holder(&me);
            w.check(check.clone());
            w.commit();
        }
    }

    // ── Views ──────────────────────────────────────────────────────

    /// Where `hash`'s bytes are, and how sure this device is of each copy.
    pub async fn locations(&self, hash: &str) -> Locations {
        self.refresh_ledgers_if_empty().await;
        let hash = hash.to_lowercase();
        let found = lock(&self.books.registry).places(&hash);
        let size = found.first().map(|f| f.row.size);
        let seen = self.seen(&hash, size);
        let me = self.holder_id();
        let mut copies: Vec<Copy> = seen
            .iter()
            .map(|s| Copy {
                holder: s.holder.clone(),
                state: s.att.state,
                confidence: s.verdict.confidence,
                verified_at_us: s.verdict.evidence_at_us,
                how: s.verdict.how.clone(),
                reachable: self.reachable(&s.holder.id),
                here: s.holder.id == me,
            })
            .collect();
        copies.sort_by(|a, b| (a.confidence, &a.holder.name).cmp(&(b.confidence, &b.holder.name)));
        let verified: Vec<&Copy> = copies
            .iter()
            .filter(|c| c.confidence == Confidence::Verified)
            .collect();
        let sites: BTreeSet<&str> = verified.iter().map(|c| c.holder.site.as_str()).collect();
        let policy = {
            let cache = lock(&self.books.cache);
            found.iter().map(|f| cache.policy(&f.org, &f.place)).fold(
                None,
                |acc: Option<Policy>, p| {
                    Some(acc.map_or(p, |a| Policy {
                        min_verified: a.min_verified.max(p.min_verified),
                        min_sites: a.min_sites.max(p.min_sites),
                    }))
                },
            )
        }
        .unwrap_or_default();
        let (v, s) = (
            u32::try_from(verified.len()).unwrap_or(u32::MAX),
            u32::try_from(sites.len()).unwrap_or(u32::MAX),
        );
        Locations {
            size: size
                .or_else(|| seen.first().map(|s| s.att.size))
                .unwrap_or(0),
            places: found
                .into_iter()
                .map(|f| Place {
                    org: f.org,
                    root: f.place,
                    path: f.row.path,
                })
                .collect(),
            hash,
            verified: v,
            sites: s,
            lacks: policy.lacks(v, s),
            policy,
            copies,
        }
    }

    /// Where the bytes of `path` in a root are.
    ///
    /// # Errors
    ///
    /// No such file.
    pub async fn locations_of(&self, doc_id: Uuid, path: &str) -> Result<Locations, EngineError> {
        let v = self
            .tree(doc_id)
            .await?
            .current(path)
            .ok_or_else(|| EngineError::NoRoot(path.to_owned()))?;
        let hash = v.content.ok_or(EngineError::NoVersion)?;
        Ok(self.locations(&hash).await)
    }

    /// Every holder this device knows, in the orgs it sees.
    pub async fn holders(&self) -> Vec<HolderStatus> {
        self.refresh_ledgers().await;
        let me = self.holder_id();
        let cache = lock(&self.books.cache);
        let mut out: BTreeMap<String, HolderStatus> = BTreeMap::new();
        out.insert(
            me.clone(),
            HolderStatus {
                holder: self.holder(),
                reachable: true,
                here: true,
                copies: 0,
                bytes: 0,
            },
        );
        for d in cache.docs.values() {
            let LedgerDoc::Holder { holder, .. } = &d.which else {
                continue;
            };
            let record = d.ledger.holder.clone().unwrap_or_else(|| Holder {
                id: holder.clone(),
                kind: HolderKind::Device,
                name: holder.chars().take(8).collect(),
                site: String::new(),
                manager: None,
            });
            let e = out.entry(holder.clone()).or_insert_with(|| HolderStatus {
                reachable: self.reachable(holder),
                here: holder == &me,
                holder: record,
                copies: 0,
                bytes: 0,
            });
            for a in d.ledger.copies.values().filter(|a| a.state.counts()) {
                e.copies = e.copies.saturating_add(1);
                e.bytes = e.bytes.saturating_add(a.size);
            }
        }
        drop(cache);
        out.into_values().collect()
    }

    /// The copy report of `scope`: an org, a root's place, or `""` for all.
    pub async fn copy_report(&self, scope: &str) -> CopyReport {
        self.refresh_ledgers_if_empty().await;
        let window = i64::try_from(self.config().verify_window.as_micros()).unwrap_or(i64::MAX);
        let now = now_us();
        let me = self.holder_id();
        let roots: Vec<RootIndex> = {
            let mut r = lock(&self.books.registry);
            r.load();
            r.roots()
                .map(|(_, i)| i)
                .filter(|i| scope.is_empty() || i.org == scope || i.place == scope)
                .cloned()
                .collect()
        };
        let mut cache = lock(&self.books.cache);
        cache.merge_checks();
        let mut report = CopyReport {
            scope: scope.to_owned(),
            policy: Policy::default(),
            files: 0,
            bytes: 0,
            under_files: 0,
            under_bytes: 0,
            single_holder_bytes: 0,
            unverified_bytes: 0,
            holders: Vec::new(),
            violations: Vec::new(),
            violation_count: 0,
        };
        let mut shares: BTreeMap<String, HolderShare> = BTreeMap::new();
        let mut done: HashSet<(String, String)> = HashSet::new();
        for root in &roots {
            let policy = cache.policy(&root.org, &root.place);
            if root.place == scope || (roots.len() == 1 && !scope.is_empty()) {
                report.policy = policy;
            } else if root.org == scope {
                report.policy = cache.policy(&root.org, "");
            }
            let docs: Vec<&DocCache> = cache.holders_in(&root.org).collect();
            let checks = cache.checks.get(&root.org);
            for row in root.rows.iter().filter(|r| !r.content.is_empty()) {
                report.files = report.files.saturating_add(1);
                report.bytes = report.bytes.saturating_add(row.size);
                let mut verified = 0u32;
                let mut sites: BTreeSet<&str> = BTreeSet::new();
                let mut holders = 0u32;
                for d in &docs {
                    let (Some(att), LedgerDoc::Holder { holder, .. }) =
                        (d.ledger.copies.get(&row.content), &d.which)
                    else {
                        continue;
                    };
                    if !att.state.counts() {
                        continue;
                    }
                    holders = holders.saturating_add(1);
                    let of_it = checks
                        .and_then(|m| m.get(&row.content))
                        .into_iter()
                        .flatten()
                        .filter(|(by, c)| &c.holder == holder && by != holder)
                        .map(|(by, c)| (by.as_str(), c));
                    let v = confidence(att, Some(row.size), of_it, now, window);
                    let ok = v.confidence == Confidence::Verified;
                    let site = d.ledger.holder.as_ref().map_or("", |h| h.site.as_str());
                    if ok {
                        verified = verified.saturating_add(1);
                        sites.insert(site);
                    }
                    if done.insert((holder.clone(), row.content.clone())) {
                        let share = shares.entry(holder.clone()).or_insert_with(|| HolderShare {
                            holder: d.ledger.holder.clone().unwrap_or_else(|| Holder {
                                id: holder.clone(),
                                kind: HolderKind::Device,
                                name: holder.chars().take(8).collect(),
                                site: String::new(),
                                manager: None,
                            }),
                            files: 0,
                            bytes: 0,
                            verified_bytes: 0,
                            reachable: holder == &me || self.reachable(holder),
                        });
                        share.files = share.files.saturating_add(1);
                        share.bytes = share.bytes.saturating_add(row.size);
                        if ok {
                            share.verified_bytes = share.verified_bytes.saturating_add(row.size);
                        }
                    }
                }
                if holders <= 1 {
                    report.single_holder_bytes =
                        report.single_holder_bytes.saturating_add(row.size);
                }
                if verified == 0 {
                    report.unverified_bytes = report.unverified_bytes.saturating_add(row.size);
                }
                let site_count = u32::try_from(sites.len()).unwrap_or(u32::MAX);
                if let Some(lacks) = policy.lacks(verified, site_count) {
                    report.under_files = report.under_files.saturating_add(1);
                    report.under_bytes = report.under_bytes.saturating_add(row.size);
                    report.violation_count = report.violation_count.saturating_add(1);
                    if report.violations.len() < VIOLATIONS_SHOWN * 4 {
                        report.violations.push(Violation {
                            root: root.place.clone(),
                            path: row.path.clone(),
                            hash: row.content.clone(),
                            size: row.size,
                            verified,
                            sites: site_count,
                            lacks,
                        });
                    }
                }
            }
        }
        drop(cache);
        report.violations.sort_by(|a, b| {
            (a.verified, std::cmp::Reverse(a.size)).cmp(&(b.verified, std::cmp::Reverse(b.size)))
        });
        report.violations.truncate(VIOLATIONS_SHOWN);
        report.holders = shares.into_values().collect();
        report.holders.sort_by_key(|h| std::cmp::Reverse(h.bytes));
        report
    }

    // ── Policy ─────────────────────────────────────────────────────

    /// Set the policy of `scope` (an org slug, or a root's place); `None` clears it.
    ///
    /// # Errors
    ///
    /// `scope` is no org or root this device sees.
    pub async fn set_policy(&self, scope: &str, policy: Option<Policy>) -> Result<(), EngineError> {
        let (org, key) = {
            let mut r = lock(&self.books.registry);
            r.load();
            r.roots()
                .find_map(|(_, i)| {
                    if i.place == scope {
                        Some((i.org.clone(), scope.to_owned()))
                    } else if i.org == scope {
                        Some((i.org.clone(), String::new()))
                    } else {
                        None
                    }
                })
                .ok_or_else(|| EngineError::NoRoot(scope.to_owned()))?
        };
        let id = self.learn_ledger(LedgerDoc::Policy { org });
        let doc = self
            .doc(id)
            .await
            .ok_or_else(|| EngineError::NoRoot(scope.to_owned()))?;
        ledger::set_policy(&doc, &key, policy);
        self.refresh_ledgers().await;
        Ok(())
    }

    /// The policy that holds over a root's place.
    #[must_use]
    pub fn policy_for(&self, org: &str, place: &str) -> Policy {
        lock(&self.books.cache).policy(org, place)
    }

    /// Refuse to drop this device's copies of `losing` (`(place, path,
    /// hash)`) when the verified copies elsewhere would not meet policy.
    pub(super) async fn guard(
        &self,
        org: &str,
        losing: Vec<(String, String, String)>,
    ) -> Result<(), EngineError> {
        self.refresh_ledgers().await;
        let me = self.holder_id();
        let mut under = Vec::new();
        for (place, path, hash) in losing {
            let policy = self.policy_for(org, &place);
            let seen = self.seen(&hash, None);
            let others: Vec<&Seen> = seen
                .iter()
                .filter(|s| s.holder.id != me && s.verdict.confidence == Confidence::Verified)
                .collect();
            let sites: BTreeSet<&str> = others.iter().map(|s| s.holder.site.as_str()).collect();
            let (v, s) = (
                u32::try_from(others.len()).unwrap_or(u32::MAX),
                u32::try_from(sites.len()).unwrap_or(u32::MAX),
            );
            if let Some(lacks) = policy.lacks(v, s) {
                under.push(format!("{path} ({lacks} elsewhere)"));
            }
        }
        if under.is_empty() {
            Ok(())
        } else {
            under.sort();
            Err(EngineError::BelowPolicy(under))
        }
    }

    /// Drop the cached bytes of `path` (a file, or everything under it;
    /// `""` the whole root) from this device's blob cache. A pinned
    /// working copy's bytes and the node's are not a cache. How many
    /// blobs went.
    ///
    /// # Errors
    ///
    /// [`EngineError::BelowPolicy`] when dropping them would leave a file
    /// under its policy; the root is not known.
    pub async fn evict(&self, doc_id: Uuid, path: &str) -> Result<u32, EngineError> {
        if self.config().hold_all {
            return Err(EngineError::Io(
                "the node holds every root: nothing here is a cache".to_owned(),
            ));
        }
        let meta = self
            .meta(doc_id)
            .ok_or_else(|| EngineError::NoRoot(doc_id.to_string()))?;
        let tree = self.tree(doc_id).await?;
        let prefix = format!("{}/", path.trim_matches('/'));
        let mut losing = Vec::new();
        let mut hashes = Vec::new();
        for (p, v) in tree.live() {
            if !(path.is_empty() || p == path || p.starts_with(&prefix)) {
                continue;
            }
            let Some(hash) = v.content.as_deref().and_then(|c| Sha256::from_hex(c).ok()) else {
                continue;
            };
            if !self.store.local.path_for(&hash).is_file()
                || self.store.in_place.find(&hash).is_some()
            {
                continue;
            }
            losing.push((meta.place(), p.clone(), hash.to_hex()));
            hashes.push(hash);
        }
        self.guard(&meta.org, losing).await?;
        // Evicting first, so a reader never sees a copy claimed that is going.
        let evicting: BTreeMap<String, Option<Holding>> = hashes
            .iter()
            .filter_map(|h| {
                let size = std::fs::metadata(self.store.local.path_for(h)).ok()?.len();
                Some((
                    h.to_hex(),
                    Some(Holding {
                        state: CopyState::Evicting,
                        size,
                        have: size,
                    }),
                ))
            })
            .collect();
        self.write_own(&meta.org, &evicting).await;
        for h in &hashes {
            let _ = self.store.local.remove(h);
            let _ = std::fs::remove_file(self.merkle_path(&h.to_hex()));
        }
        self.attest(&hashes).await;
        Ok(u32::try_from(hashes.len()).unwrap_or(u32::MAX))
    }

    /// Drop one blob from this device's cache (the drive's eviction):
    /// refused under policy like [`Self::evict`], attested `evicting`
    /// first and withdrawn once gone. A pinned working copy's bytes and
    /// the node's are not a cache.
    ///
    /// # Errors
    ///
    /// [`EngineError::BelowPolicy`], or the file cannot be removed.
    pub async fn evict_blob(&self, hash: Sha256) -> Result<(), EngineError> {
        if self.config().hold_all || self.store.in_place.find(&hash).is_some() {
            return Err(EngineError::Io(
                "not a cached copy: the working copy or the node holds it".to_owned(),
            ));
        }
        self.drop_stored(hash).await
    }

    /// Who holds a version's bytes (`content`, hex): this device, and the
    /// holders on record.
    pub async fn version_bytes(
        &self,
        version: crate::id::VersionId,
        content: &str,
    ) -> crate::roots::VersionBytes {
        let here = Sha256::from_hex(content).is_ok_and(|h| self.holds(&h));
        let holders = self.locations(content).await.copies.len();
        crate::roots::VersionBytes {
            version,
            here,
            holders: u32::try_from(holders).unwrap_or(u32::MAX),
        }
    }

    /// Move bad bytes out of a store area so they are never served: to
    /// `<data>/corrupt/`, or (an area on another filesystem, the tree's)
    /// to `<area>/.corrupt/`. Never deleted.
    pub(super) fn quarantine_file(&self, path: &std::path::Path, hex: &str) {
        let to = self.config().data.join("corrupt").join(hex);
        if let Some(dir) = to.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::rename(path, &to).is_ok() {
            return;
        }
        let beside = self
            .store
            .areas()
            .into_iter()
            .find(|a| path.starts_with(a.root()))
            .map(|a| a.root().join(".corrupt").join(hex));
        if let Some(beside) = beside {
            if let Some(dir) = beside.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::rename(path, &beside);
        }
    }

    /// Bad bytes found in a store area (`path`, claimed as `hash`):
    /// quarantined, the claim withdrawn, counted as damaged.
    pub(super) async fn quarantine(&self, hash: Sha256, path: &std::path::Path) {
        let hex = hash.to_hex();
        self.quarantine_file(path, &hex);
        let _ = std::fs::remove_file(self.merkle_path(&hex));
        lock(&self.books.damaged).insert(hex.clone());
        let orgs = lock(&self.books.registry).orgs_of(&hex);
        let gone: BTreeMap<String, Option<Holding>> = std::iter::once((hex, None)).collect();
        for org in &orgs {
            self.write_own(org, &gone).await;
        }
        self.attest(&[hash]).await;
    }

    /// Every content that is an old version somewhere (not what a path's
    /// current version holds): never evicted for space, whatever else
    /// holds the same bytes now.
    pub(super) async fn old_contents(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        for id in self.known() {
            let Ok(tree) = self.tree(id).await else {
                // A root it cannot read: nothing of it is known to be safe.
                continue;
            };
            for path in tree.paths() {
                let current = tree.current(path).map(|v| v.id);
                for v in tree.history(path) {
                    if Some(v.id) != current
                        && let Some(c) = &v.content
                    {
                        out.insert(c.clone());
                    }
                }
            }
        }
        out
    }

    /// Drop one blob from the store to make room (the store's budget,
    /// [`super::space`]): only a blob whose bytes this same device holds
    /// in a working file in place, re-hashed from that file just before
    /// the delete, and only if the bytes are no old version anywhere (a
    /// working file can change, so it is never where a version is kept).
    /// Holds the pass's lock, so a write deciding the store keeps these
    /// bytes never interleaves with it.
    ///
    /// Nothing else is evicted for space: a copy only other holders keep
    /// would need the org's policy applied to old versions, a live
    /// possession check just before the delete, and an `evicting` note
    /// the other holders have seen first, or two holders can each drop
    /// the last two copies (#530).
    ///
    /// # Errors
    ///
    /// [`EngineError::BelowPolicy`]: pinned, an old version, or no working
    /// file holds these bytes now; or the file cannot be read or removed.
    pub async fn evict_for_space(&self, hash: Sha256) -> Result<(), EngineError> {
        let _busy = self.busy().await;
        let old = self.old_contents().await;
        self.evict_for_space_locked(hash, &old).await
    }

    /// [`Self::evict_for_space`] with the lock held and the old versions
    /// known.
    pub(super) async fn evict_for_space_locked(
        &self,
        hash: Sha256,
        old: &HashSet<String>,
    ) -> Result<(), EngineError> {
        if self.store.local.is_pinned(hash).await.unwrap_or(true) {
            return Err(EngineError::BelowPolicy(vec![format!("{hash} (pinned)")]));
        }
        if old.contains(&hash.to_hex()) {
            return Err(EngineError::BelowPolicy(vec![format!(
                "{hash} (an old version: never evicted for space)"
            )]));
        }
        let Some(working) = self.store.in_place.find(&hash) else {
            return Err(EngineError::BelowPolicy(vec![format!(
                "{hash} (no working file holds it: never evicted for space, #530)"
            )]));
        };
        let (_, actual) = architect::blob::hash_file(&working).await?;
        if actual != hash {
            // The index's stat lied (an edit that kept size and mtime).
            self.store.in_place.forget(&hash);
            return Err(EngineError::Changed(working.display().to_string()));
        }
        // Still the same file after the read.
        if self.store.in_place.find(&hash).as_deref() != Some(working.as_path()) {
            return Err(EngineError::Changed(working.display().to_string()));
        }
        self.store.local.remove(&hash)?;
        Ok(())
    }

    /// [`Self::evict_blob`]'s work: the policy guard, `evicting`, gone.
    async fn drop_stored(&self, hash: Sha256) -> Result<(), EngineError> {
        let stored = self.store.local.path_for(&hash);
        let Ok(meta) = std::fs::metadata(&stored) else {
            return Ok(());
        };
        let hex = hash.to_hex();
        let places = lock(&self.books.registry).places(&hex);
        let mut by_org: BTreeMap<String, Vec<(String, String, String)>> = BTreeMap::new();
        for f in places {
            by_org
                .entry(f.org)
                .or_default()
                .push((f.place, f.row.path, hex.clone()));
        }
        for (org, losing) in by_org {
            self.guard(&org, losing).await?;
        }
        let size = meta.len();
        let evicting: BTreeMap<String, Option<Holding>> = std::iter::once((
            hex.clone(),
            Some(Holding {
                state: CopyState::Evicting,
                size,
                have: size,
            }),
        ))
        .collect();
        let orgs = lock(&self.books.registry).orgs_of(&hex);
        for org in &orgs {
            self.write_own(org, &evicting).await;
        }
        self.store.local.remove(&hash)?;
        let _ = std::fs::remove_file(self.merkle_path(&hex));
        self.attest(&[hash]).await;
        Ok(())
    }

    /// The hashes of a root's working copy that would leave this device
    /// on an unpin (not also kept in the store), for [`Self::guard`].
    pub(super) fn leaving_on_unpin(
        &self,
        place: &str,
        index: &Index,
    ) -> Vec<(String, String, String)> {
        index
            .files
            .iter()
            .filter(|(_, i)| {
                Sha256::from_hex(&i.content).is_ok_and(|h| !self.store.local.path_for(&h).is_file())
            })
            .map(|(p, i)| (place.to_owned(), p.clone(), i.content.clone()))
            .collect()
    }

    /// Everything [`Engine::pass_all`] does for the copies after its passes.
    pub(super) async fn keep_books(&self) {
        self.refresh_registry().await;
        self.refresh_ledgers().await;
        self.keep_store().await;
        self.reconcile().await;
        let budget = self.config().scrub_budget;
        if budget > 0 {
            let s = self.scrub(None, budget).await;
            if !s.corrupt.is_empty() {
                tracing::warn!(
                    corrupt = s.corrupt.len(),
                    "files: scrub removed bad copies from the count"
                );
            }
        }
        let c = self.challenge(self.config().challenges, None).await;
        if !c.failed.is_empty() {
            tracing::warn!(failed = ?c.failed, "files: a peer's copy failed its challenge");
        }
    }
}

/// The keys of the ledger's maps that changed between two frontiers.
fn changed_keys(
    doc: &LoroDoc,
    from: &Frontiers,
    to: &Frontiers,
) -> Option<BTreeMap<String, BTreeSet<String>>> {
    let diff = doc.diff(from, to).ok()?;
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (cid, d) in diff.iter() {
        let loro::ContainerID::Root { name, .. } = cid else {
            return None;
        };
        let loro::event::Diff::Map(m) = d else {
            return None;
        };
        out.entry(name.to_string())
            .or_default()
            .extend(m.updated.keys().map(ToString::to_string));
    }
    Some(out)
}

fn signer_of(which: &LedgerDoc, holder: Option<&Holder>) -> String {
    match which {
        LedgerDoc::Holder { holder: id, .. } => holder
            .filter(|h| &h.id == id)
            .and_then(|h| h.manager.clone())
            .unwrap_or_else(|| id.clone()),
        LedgerDoc::Policy { .. } => String::new(),
    }
}

/// Read a ledger document into its cache entry: only `changed` keys when
/// known, the whole document otherwise. A record whose signature does not
/// verify is left out.
fn read_doc(
    doc: &LoroDoc,
    which: LedgerDoc,
    frontiers: Frontiers,
    old: Option<DocCache>,
    changed: Option<BTreeMap<String, BTreeSet<String>>>,
) -> DocCache {
    if let LedgerDoc::Policy { .. } = which {
        return DocCache {
            which,
            frontiers,
            ledger: HolderLedger::default(),
            policies: ledger::policies(doc),
        };
    }
    let org = which.org().to_owned();
    let holder_id = match &which {
        LedgerDoc::Holder { holder, .. } => holder.clone(),
        LedgerDoc::Policy { .. } => String::new(),
    };
    match (old, changed) {
        (Some(mut old), Some(keys)) if !keys.contains_key("holder") => {
            let signer = signer_of(&which, old.ledger.holder.as_ref());
            let copies = doc.get_map("copies");
            for k in keys.get("copies").into_iter().flatten() {
                let att = copies
                    .get(k)
                    .and_then(|v| v.into_value().ok())
                    .and_then(|v| match v {
                        LoroValue::String(s) => facet_json::from_str::<Attestation>(&s).ok(),
                        _ => None,
                    })
                    .filter(|a| ledger::verify(&signer, &a.message(&org, &holder_id), &a.sig));
                match att {
                    Some(a) => {
                        old.ledger.copies.insert(k.clone(), a);
                    }
                    None => {
                        old.ledger.copies.remove(k);
                    }
                }
            }
            let checks = doc.get_map("checks");
            for k in keys.get("checks").into_iter().flatten() {
                let check = checks
                    .get(k)
                    .and_then(|v| v.into_value().ok())
                    .and_then(|v| match v {
                        LoroValue::String(s) => facet_json::from_str::<Check>(&s).ok(),
                        _ => None,
                    })
                    .filter(|c| ledger::verify(&holder_id, &c.message(&org, &holder_id), &c.sig));
                if let Some(c) = check {
                    old.ledger
                        .checks
                        .insert((c.holder.clone(), c.hash.clone()), c);
                }
            }
            old.frontiers = frontiers;
            old
        }
        _ => {
            let mut ledger = ledger::read(doc);
            let signer = signer_of(&which, ledger.holder.as_ref());
            verify_all(&mut ledger, &org, &holder_id, &signer);
            DocCache {
                which,
                frontiers,
                ledger,
                policies: BTreeMap::new(),
            }
        }
    }
}

/// Check every record's signature, on every core.
fn verify_all(ledger: &mut HolderLedger, org: &str, holder: &str, signer: &str) {
    let copies: Vec<(String, Attestation)> =
        std::mem::take(&mut ledger.copies).into_iter().collect();
    let threads = std::thread::available_parallelism()
        .map_or(4, std::num::NonZero::get)
        .min(12);
    let per = copies.len().div_ceil(threads).max(1);
    let kept: Vec<Vec<(String, Attestation)>> = std::thread::scope(|s| {
        // Collected, so every thread is running before the first is joined.
        #[allow(clippy::needless_collect)]
        let handles: Vec<_> = copies
            .chunks(per)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .filter(|(_, a)| ledger::verify(signer, &a.message(org, holder), &a.sig))
                        .cloned()
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    ledger.copies = kept.into_iter().flatten().collect();
    ledger
        .checks
        .retain(|_, c| ledger::verify(holder, &c.message(org, holder), &c.sig));
}

/// Another engine in the same process as a prover (tests, two apps on one disk).
impl Prover for Engine {
    fn prove(
        &self,
        hash: Sha256,
        index: u64,
    ) -> futures_util::future::BoxFuture<'_, Result<Proof, verify::ProveError>> {
        Box::pin(async move { Self::prove(self, hash, index).await.map_err(Into::into) })
    }
}

/// [`verify::Possession`] over an engine: what a node serves on its peer
/// lane, proving only blobs some manifest it holds names.
#[derive(Clone)]
pub struct PossessionHost(pub Engine);

impl verify::Possession for PossessionHost {
    async fn prove(&self, hash: String, index: u64) -> Result<Proof, PossessionError> {
        let hash = Sha256::from_hex(&hash).map_err(|_| PossessionError::BadHash(hash.clone()))?;
        Engine::prove(&self.0, hash, index).await
    }
}
