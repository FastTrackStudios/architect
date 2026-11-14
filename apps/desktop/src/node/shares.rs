//! Share links on the node: `<node>/s/<code>/` (#374, the model is #197's;
//! the `/s` prefix is [`architect_spaces::share::LINKS`], #431).
//!
//! A share ([`architect_spaces::Share`]) is a note of the vault it shares
//! (`_architect/shares/<code>.md`, hidden from every Site), so it syncs
//! with the vault and every node that holds the vault knows it. This node:
//!
//! - **keeps the table** ([`Shares`]): it reads the shares of each vault it
//!   holds at start and again whenever the vault changes, so a share made
//!   or revoked on another device reaches it with the note;
//! - **builds each live share** as a scoped Site (`architect_sites::Scope`)
//!   at `/s/<code>/` into its data dir, and again whenever the vault changes,
//!   so an edit (from disk, a peer's live bridge or a web client) is on the
//!   shared page within a second or so; the page reloads itself (`_live`,
//!   an event stream);
//! - **answers `/s/<code>/…`** ([`gate`]) only while the share is live:
//!   expired, revoked or unknown is `404`. Every request is checked against
//!   the share's audience: anyone for a public or link share, and for an
//!   org or invited share a person signed in at the issuer (their session
//!   as the bearer, or the `architect_session` cookie the node's sign-in
//!   sets), resolved with the issuer's `whoami` and `orgs`. A share with
//!   a domain of its own is served at that host's root too;
//! - **serves [`ShareLinks`]** on the org lane, for the app: make, list,
//!   change and revoke. A revoke takes the link down before it answers.
//!
//! A share's named audiences (#510) are each a filter over its scope on a
//! link of their own (`/s/<the audience's code>/`). Each is built as its
//! own site, of the notes its filter holds for alone, so nothing else is
//! listed on it (pages, nav, backlinks, base rows, search, tags). And
//! each request to it is checked when it comes ([`Guard`]): the page or
//! file asked for is served only if, in the vault as it is now, its note
//! is in the scope and the filter holds for it (a file: for a note that
//! attaches it). Anything else of the share is `404` there, on the page
//! route and the byte route alike, whatever is on disk.
//!
//! Every link has a files page (`<link>_files/`, #510): what the link
//! lists, read from the vault when it is asked for, in list order, by
//! note or by a property (`?by=order|note|property:<key>`), each view a
//! link on the page. "Download all" there, and on each group, is
//! `<link>_archive.zip?by=…[&group=…]`: a zip of the files in view, in
//! folders by that grouping and numbered in list order, written to the
//! response as the files are read ([`architect_sites::archive`]): never
//! whole in memory, never on disk. An audience's lists and archives are
//! its filter's, as its pages are.
//!
//! A link plays (#511): every file but a page is streamed with ranges
//! (`206`, `Accept-Ranges`, `Content-Range`), behind the same checks as
//! any request to the link, so a page's `<audio>` and `<video>` seek
//! without fetching the file. Each page carries its link's queue
//! ([`queue`]: the audio and video of the notes it lists, in list
//! order, read per request), which the site's player
//! (`features/sites/assets/player.js`) plays through.
//!
//! A link plays to a layer (#528, [`Listen`]: page, reference, full),
//! set on the share and narrowed per audience. The `[[share_layers]]`
//! rules of the node's extensions place each file a note of their type
//! attaches ([`LayerRules`]); a file above the link's layer is refused
//! (`403`, saying the layer it needs) on every route that hands out
//! bytes: the file itself (by its name, or any other spelling of the same
//! file on disk), the files page, the archive and the queue leave it out.
//! `<link>_files.json` lists what the link serves, for a client that
//! opens a song over it (Studio): each file's size, hash, layer and URL,
//! the link's layer and the layer asked for (`?listen=`, never past the
//! link's). A file no rule places is a page file, served as before.
//!
//! Nothing outside a share's scope is ever built into it, so nothing
//! outside it can be served: the site is built from the scope's notes
//! alone, its links to other notes are plain text, and an attachment is
//! there only when a note in the scope links to it.
//!
//! Sign-in (`/s/_signin`) is the issuer's device sign-in: the node
//! starts one, the person approves it at the issuer, signed in there,
//! and the node sets its session as an `HttpOnly` cookie. The node never
//! sees a password.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use architect::http::axum::body::Body;
use architect::http::axum::extract::{Request, State};
use architect::http::axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use architect::http::axum::middleware::Next;
use architect::http::axum::response::sse::{Event, KeepAlive, Sse};
use architect::http::axum::response::{IntoResponse, Redirect, Response};
use architect_orgs::server::{Directory as _, IdentityServer};
use architect_permissions::Principal;
use architect_sites::archive::ZipWriter;
use architect_sites::listing::{self, Entry, Group, Grouping, Vault};
use architect_spaces::share::{
    LINKS, SHARES_FOLDER, code_from, comments_note_path, is_code, link_path, share_note_path,
    validate, validate_audience, validate_domain,
};
use architect_spaces::{
    GuestFault, GuestLink, GuestNote, LayersReport, ListedFile, ListedNote, Listen, NewAudience,
    NewShare, Share, ShareAudience, ShareChange, ShareComment, ShareFault, ShareGuest, ShareLinks,
    ShareScope, ShareState, Verdict, Viewer, parse_comments, parse_share_note, share_note,
};
use tokio::sync::{Notify, watch};
use vault_live::Backend;
use vault_proto::{IfMatch, VaultSync as _};

use super::org::{IssuerLane, NodeOrg};
use super::share_layers::{LayerRules, same_file, within};
use super::share_shell;

/// The cookie a signed-in viewer's issuer session is kept in.
pub const SESSION_COOKIE: &str = "architect_session";
/// The comments section's start: its look, then its heading. The site's
/// own tokens (`--site-*`) colour it, so it follows the theme.
/// The cookie a sign-in in progress is kept in, and its path.
const SIGNIN_COOKIE: &str = "architect_signin";
const SIGNIN_PATH: &str = "/s/_signin";
/// How long the vault is left to settle before the shares are rebuilt.
const SETTLE: Duration = Duration::from_millis(400);
/// How often the layer rules are looked at for a change to build for.
const RULES_EVERY: Duration = Duration::from_secs(2);
/// How long a resolved viewer is trusted before the issuer is asked again.
const VIEWER_TTL: Duration = Duration::from_secs(60);

/// Set by [`fail_vault_views_for_tests`]: every vault view is unreadable.
static VIEWS_FAIL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Make every vault view unreadable (`true`) or readable again: a test's
/// way to hold the moment a vault cannot be read (#540's third review).
#[doc(hidden)]
pub fn fail_vault_views_for_tests(fail: bool) {
    VIEWS_FAIL.store(fail, Ordering::SeqCst);
}

/// Unix seconds now.
#[must_use]
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// A share as the node holds it: the share and how far its pages are
/// built (`0`: not yet; each build counts up, and the pages' event
/// stream fires).
struct Held {
    share: Share,
    built: watch::Sender<u64>,
    /// What each of its links' built pages are of, by the link's code
    /// (its own, and each audience's).
    manifests: HashMap<String, Arc<Manifest>>,
}

impl Held {
    fn new(share: Share) -> Self {
        Self {
            share,
            built: watch::channel(0).0,
            manifests: HashMap::new(),
        }
    }
}

/// What a built link's files are of: which note each page is, which
/// attachment each copied file. A request is checked through it.
#[derive(Default)]
struct Manifest {
    /// A page's path below the link (`/joy/`) to its note's vault path.
    pages: HashMap<String, String>,
    /// A file's path below the link (`/Set/joy.pdf`) to its vault path.
    files: HashMap<String, String>,
}

impl Manifest {
    fn of(site: &architect_sites::Site) -> Self {
        let mut manifest = Self::default();
        for page in &site.pages {
            if let Some(note) = &page.rel_path {
                let below = page.url.strip_prefix(&site.base).unwrap_or(&page.url);
                manifest
                    .pages
                    .insert(format!("/{}", below.trim_start_matches('/')), note.clone());
            }
        }
        for file in &site.attachments {
            manifest.files.insert(format!("/{file}"), file.clone());
        }
        manifest
    }

    /// The path below the link of `note`'s page.
    fn page_of(&self, note: &str) -> Option<&str> {
        self.pages
            .iter()
            .find(|(_, n)| n.as_str() == note)
            .map(|(path, _)| path.as_str())
    }
}

/// A vault as requests are checked against it: read once per change.
struct VaultView {
    generation: u64,
    vault: Vault,
    /// What each link lists in this vault, once asked.
    listings: Mutex<HashMap<String, Arc<Vec<Entry>>>>,
    /// Every note, hidden ones too, with every file it links: what
    /// places a file in a layer (#528), once asked.
    notes: std::sync::OnceLock<Vec<Entry>>,
    /// Where each file is placed, by the rules' generation it was placed
    /// with (None inside: not known).
    placed: Mutex<Option<(u64, Option<Arc<HashMap<String, Listen>>>)>>,
    /// Each placed file by its folded path (`share_layers::fold`, `//`
    /// collapsed): how another spelling of it is found. By the rules'
    /// generation, as `placed`.
    folded: Mutex<
        Option<(
            Arc<HashMap<String, Listen>>,
            Arc<HashMap<String, Vec<String>>>,
        )>,
    >,
    /// What of the vault did not read (a note, a folder): while there is
    /// any, where files are is not known (#540's third review).
    unread: Vec<String>,
}

/// A vault's view, and the lock one reader holds while it reads it.
type ViewSlot = Arc<Mutex<Option<Arc<VaultView>>>>;

/// Whether a file is served on a link below Full ([`Shares::above`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// Serve it (or its 404).
    Serve,
    /// Held back: it needs this layer.
    Held(Listen),
    /// Above the layer and not one of the link's files: answered as a
    /// path that is nowhere.
    Hidden,
    /// The folder is being swapped in, or the file cannot be read: held
    /// back, as "being built".
    Building,
}

/// What a request to an audience's link is answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Guard {
    /// Serve it.
    Open,
    /// A note or a file the audience's filter does not hold for.
    Refused,
    /// Its pages are not built yet.
    Building,
    /// Its filter holds for nothing.
    Empty,
}

/// The node's share links. Cheap to clone.
#[derive(Clone)]
pub struct Shares(Arc<Inner>);

struct Inner {
    backend: Backend,
    /// `<data>/shares`: each share's pages under its code.
    out: PathBuf,
    org: Option<NodeOrg>,
    issuer: Option<IssuerLane>,
    issuer_origin: Option<String>,
    table: RwLock<HashMap<String, Held>>,
    viewers: Mutex<HashMap<String, (Viewer, Instant)>>,
    signins: Mutex<HashMap<String, Pending>>,
    /// Per vault: rebuild now.
    kick: Mutex<HashMap<String, Arc<Notify>>>,
    /// What draws the fences extensions claim (#488).
    blocks: architect_sites::Blocks,
    /// Which layer each attached file is in (#528).
    layers: LayerRules,
    /// Per vault: how many times it has changed. A view of it older
    /// than this is read again before a request is checked against it.
    moved: Mutex<HashMap<String, Arc<AtomicU64>>>,
    views: Mutex<HashMap<String, ViewSlot>>,
    /// The islands its pages' players may draw (studio#445), served at
    /// `/_ext/` (`architect_sites::island`).
    islands: std::sync::OnceLock<Arc<[architect_sites::SiteIsland]>>,
    /// Note types' favourites (`[[favourites]]`), each with the extension
    /// that declared it: the web app's listing.
    favourites: std::sync::OnceLock<Arc<[architect_favourites::Listed]>>,
    /// Whether the web app is served beside the node: a link's page is
    /// the app's, or says it needs it ([`share_shell::WebApp`]).
    web_app: std::sync::OnceLock<share_shell::WebApp>,
}

/// A device sign-in a browser is waiting on.
struct Pending {
    device_code: String,
    to: String,
    until: Instant,
}

impl Shares {
    /// The shares of `backend`'s vaults, built under `data/shares`, their
    /// pages drawing the fences `blocks` claim as the app draws them
    /// (#488), their files placed in layers by `layers` (#528).
    #[must_use]
    pub fn new(
        backend: Backend,
        data: &Path,
        org: Option<NodeOrg>,
        issuer: Option<&str>,
        blocks: architect_sites::Blocks,
        layers: LayerRules,
    ) -> Self {
        Self(Arc::new(Inner {
            backend,
            out: data.join("shares"),
            org,
            issuer: issuer.map(IssuerLane::new),
            issuer_origin: issuer.map(|i| i.trim_end_matches('/').to_owned()),
            table: RwLock::new(HashMap::new()),
            viewers: Mutex::new(HashMap::new()),
            signins: Mutex::new(HashMap::new()),
            kick: Mutex::new(HashMap::new()),
            blocks,
            layers,
            moved: Mutex::new(HashMap::new()),
            views: Mutex::new(HashMap::new()),
            islands: std::sync::OnceLock::new(),
            favourites: std::sync::OnceLock::new(),
            web_app: std::sync::OnceLock::new(),
        }))
    }

    /// With the web app beside it (`ARCHITECT_WEB_APP`): a link's pages
    /// are the app's ([`share_shell`]); without, they say the link needs
    /// the web app. The first call holds.
    #[must_use]
    pub fn with_web_app(self, web: share_shell::WebApp) -> Self {
        let _ = self.0.web_app.set(web);
        self
    }

    fn web_app(&self) -> share_shell::WebApp {
        self.0.web_app.get().copied().unwrap_or_default()
    }

    /// With `islands` (studio#445): its pages' players may draw them; the
    /// first call holds.
    #[must_use]
    pub fn with_islands(self, islands: Vec<architect_sites::SiteIsland>) -> Self {
        let _ = self.0.islands.set(islands.into());
        self
    }

    fn islands(&self) -> &[architect_sites::SiteIsland] {
        self.0.islands.get().map_or(&[], |i| i)
    }

    /// With note types' `favourites`, each with the extension that
    /// declared it, for the web app (`/_ext/favourites.json`); the first
    /// call holds.
    #[must_use]
    pub fn with_favourites(self, favourites: Vec<architect_favourites::Listed>) -> Self {
        let _ = self.0.favourites.set(favourites.into());
        self
    }

    fn listed_favourites(&self) -> &[architect_favourites::Listed] {
        self.0.favourites.get().map_or(&[], |f| f)
    }

    /// Every share, as the table holds it now.
    #[must_use]
    pub fn list(&self) -> Vec<Share> {
        let mut out: Vec<Share> = {
            let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
            table.values().map(|h| h.share.clone()).collect()
        };
        out.sort_by(|a, b| (a.created_at, &a.code).cmp(&(b.created_at, &b.code)));
        out
    }

    /// Share `code`, if the table holds it.
    #[must_use]
    pub fn get(&self, code: &str) -> Option<Share> {
        let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
        table.get(code).map(|h| h.share.clone())
    }

    fn built(&self, code: &str) -> Option<watch::Receiver<u64>> {
        let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
        table.get(code).map(|h| h.built.subscribe())
    }

    fn kick(&self, vault: &str) -> Arc<Notify> {
        let mut kick = self.0.kick.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(kick.entry(vault.to_owned()).or_default())
    }

    /// Read `vault`'s share notes into the table: new ones in, changed
    /// ones replaced, ones whose note is gone out.
    fn load(&self, vault: &str) {
        let mut found: HashMap<String, Share> = HashMap::new();
        if let Ok(manifest) = self.0.backend.manifest(vault) {
            for file in manifest.files {
                let Some(name) = file
                    .path
                    .strip_prefix(SHARES_FOLDER)
                    .and_then(|r| r.strip_prefix('/'))
                    .and_then(|r| r.strip_suffix(".md"))
                else {
                    continue;
                };
                if !is_code(name) {
                    continue;
                }
                let Ok(bytes) = self.0.backend.get_file(vault, &file.path) else {
                    continue;
                };
                match parse_share_note(&String::from_utf8_lossy(&bytes.0)) {
                    // A note claiming another vault, or another code, is
                    // not that share.
                    Some(mut share) if share.code == name && share.vault == vault => {
                        // An audience's link is its own: never a code
                        // that is not one, the share's, or one twice.
                        let mut seen = std::collections::HashSet::new();
                        share.audiences.retain(|a| {
                            is_code(&a.code) && a.code != name && seen.insert(a.code.clone())
                        });
                        found.insert(share.code.clone(), share);
                    }
                    _ => tracing::debug!(%vault, path = %file.path, "not a share note"),
                }
            }
        }
        let mut table = self.0.table.write().unwrap_or_else(PoisonError::into_inner);
        table.retain(|code, held| held.share.vault != vault || found.contains_key(code));
        for (code, share) in found {
            match table.get_mut(&code) {
                Some(held) => held.share = share,
                None => {
                    table.insert(code, Held::new(share));
                }
            }
        }
    }

    /// Build every live share of `vault`, remove the pages of the rest.
    fn build_all(&self, vault: &str) {
        let now = now();
        let shares: Vec<Share> = {
            let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
            table
                .values()
                .filter(|h| h.share.vault == vault)
                .map(|h| h.share.clone())
                .collect()
        };
        for share in shares {
            if share.state(now) == ShareState::Live {
                self.build(&share);
            } else {
                let _ = std::fs::remove_dir_all(self.0.out.join(&share.code));
            }
        }
        // Pages of a share no longer in the table.
        if let Ok(dirs) = std::fs::read_dir(&self.0.out) {
            let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
            for dir in dirs.flatten() {
                let name = dir.file_name().to_string_lossy().into_owned();
                if !table.contains_key(&name) {
                    let _ = std::fs::remove_dir_all(dir.path());
                }
            }
        }
    }

    /// Build `share`'s pages: at `/s/<code>/`, and at `/` for its domain.
    fn build(&self, share: &Share) {
        let Ok(root) = self.0.backend.root(&share.vault) else {
            tracing::warn!(code = %share.code, vault = %share.vault, "a share of a vault this node does not hold");
            return;
        };
        let at = self.0.out.join(&share.code);
        let mut manifests: HashMap<String, Arc<Manifest>> = HashMap::new();
        // Each link names no file above how far it plays (#528). While the
        // vault cannot be read, nothing is built: the folders stay as they
        // were, and a link below Full answers nothing from them.
        let caps: BTreeSet<Listen> = std::iter::once(share.listen)
            .chain(share.audiences.iter().map(|a| share.listen_for(Some(a))))
            .collect();
        let mut held: HashMap<Listen, BTreeSet<String>> = HashMap::new();
        for cap in caps {
            let Some(set) = self.held_back(share, cap) else {
                tracing::warn!(code = %share.code, vault = %share.vault, "a shared vault does not read: its links are not built again until it does");
                return;
            };
            held.insert(cap, set);
        }
        let options = |base: String, filter: Option<String>, cap: Listen| {
            let mut opts =
                architect_sites::Options::new(&root, share.title.clone().unwrap_or_default());
            opts.live_reload = Some(format!("{base}_live"));
            opts.base = base;
            opts.scope = scope(&share.scope);
            opts.filter = filter;
            // Each page's plain blocks, for the link's guests (#629).
            opts.page_data = true;
            if let Some(set) = held.get(&cap) {
                opts.hold_back.clone_from(set);
            }
            opts
        };
        // Its own link: the whole scope. And its domain's root.
        let own = options(share.path(), None, share.listen);
        if let Some(manifest) = self.build_one(share, &own, &at.join("site")) {
            manifests.insert(share.code.clone(), Arc::new(manifest));
        }
        if share.domain.is_some() {
            let domain = options("/".to_owned(), None, share.listen);
            self.build_one(share, &domain, &at.join("domain"));
        } else {
            let _ = std::fs::remove_dir_all(at.join("domain"));
            let _ = std::fs::remove_dir_all(architect_sites::data_dir(&at.join("domain")));
        }
        // Each audience: the notes its filter holds for, and no others.
        for audience in &share.audiences {
            let out = self.audience_dir(&share.code, &audience.code).join("site");
            let opts = options(
                audience.path(),
                Some(audience.filter.clone()),
                share.listen_for(Some(audience)),
            );
            // An audience nothing is in has no pages, and says so.
            let manifest = self.build_one(share, &opts, &out).unwrap_or_default();
            manifests.insert(audience.code.clone(), Arc::new(manifest));
        }
        if let Ok(dirs) = std::fs::read_dir(at.join("audiences")) {
            for dir in dirs.flatten() {
                let name = dir.file_name().to_string_lossy().into_owned();
                if share.audience_at(&name).is_none() {
                    let _ = std::fs::remove_dir_all(dir.path());
                }
            }
        }
        let mut table = self.0.table.write().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = table.get_mut(&share.code) {
            held.manifests = manifests;
            held.built.send_modify(|v| *v = v.wrapping_add(1));
        }
    }

    /// Build one site of `share` into `out`; what its files are of. A
    /// site that does not build (nothing in it) leaves no pages there.
    fn build_one(
        &self,
        share: &Share,
        opts: &architect_sites::Options,
        out: &Path,
    ) -> Option<Manifest> {
        let built = architect_sites::Site::read(opts, &self.0.blocks)
            .map_err(|e| e.to_string())
            .and_then(|site| {
                let report = architect_sites::write(&site, out).map_err(|e| e.to_string())?;
                Ok((Manifest::of(&site), report.pages))
            });
        match built {
            Ok((manifest, pages)) => {
                tracing::debug!(code = %share.code, base = %opts.base, pages, "a share built");
                Some(manifest)
            }
            Err(error) => {
                tracing::warn!(code = %share.code, base = %opts.base, %error, "a share did not build");
                let _ = std::fs::remove_dir_all(out);
                let _ = std::fs::remove_dir_all(architect_sites::data_dir(out));
                None
            }
        }
    }

    /// How many times `vault` has changed, as this node has heard.
    fn moved(&self, vault: &str) -> Arc<AtomicU64> {
        let mut moved = self.0.moved.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(moved.entry(vault.to_owned()).or_default())
    }

    /// `vault` as it is now: read again if it has changed since it was
    /// last read. Blocking (it reads the vault's notes).
    fn view(&self, vault: &str) -> Option<Arc<VaultView>> {
        if VIEWS_FAIL.load(Ordering::SeqCst) {
            return None;
        }
        let slot: ViewSlot = {
            let mut views = self.0.views.lock().unwrap_or_else(PoisonError::into_inner);
            Arc::clone(views.entry(vault.to_owned()).or_default())
        };
        // One reader at a time: the rest wait for its view.
        let mut held = slot.lock().unwrap_or_else(PoisonError::into_inner);
        let generation = self.moved(vault).load(Ordering::SeqCst);
        if let Some(view) = held.as_ref().filter(|v| v.generation == generation) {
            return Some(Arc::clone(view));
        }
        let root = self.0.backend.root(vault).ok()?;
        let (read, unread) = listing::open_reporting(&root)
            .map_err(|error| tracing::warn!(%vault, %error, "a shared vault could not be read"))
            .ok()?;
        let view = Arc::new(VaultView {
            generation,
            vault: read,
            listings: Mutex::new(HashMap::new()),
            notes: std::sync::OnceLock::new(),
            placed: Mutex::new(None),
            folded: Mutex::new(None),
            unread,
        });
        *held = Some(Arc::clone(&view));
        drop(held);
        Some(view)
    }

    /// What a link of `share` lists now: its scope's notes, or the ones
    /// `audience`'s filter holds for, in list order, each with its
    /// files. Blocking ([`Self::view`]).
    fn listing(&self, share: &Share, audience: Option<&ShareAudience>) -> Arc<Vec<Entry>> {
        let Some(view) = self.view(&share.vault) else {
            return Arc::default();
        };
        let filter = audience.map(|a| a.filter.as_str()).unwrap_or_default();
        let key = format!(
            "{}\n{:?}\n{filter}",
            audience.map_or(&share.code, |a| &a.code),
            share.scope
        );
        let listings = || view.listings.lock().unwrap_or_else(PoisonError::into_inner);
        let found = listings().get(&key).cloned();
        if let Some(found) = found {
            return found;
        }
        // A filter that does not parse holds for nothing.
        let entries = listing::parse_filter(filter)
            .map(|filter| listing::select(&view.vault, &scope(&share.scope), &filter))
            .unwrap_or_default();
        let entries = Arc::new(entries);
        listings().insert(key, Arc::clone(&entries));
        entries
    }

    /// The layer of each file of `share`'s vault above the page, as the
    /// vault and the rules are now: placed by every note that links it,
    /// in the share's scope or not, hidden or not, so a note outside the
    /// scope never lowers it. Placed once per vault change or rules
    /// change, never per request. None when it is not known (a manifest
    /// or the vault does not read): hold everything back. Blocking
    /// ([`Self::view`]).
    fn placed(&self, share: &Share) -> Option<Arc<HashMap<String, Listen>>> {
        let current = self.0.layers.current();
        // Placed with no rules too: a file only a media item's properties
        // attach is the reference's by default (#668).
        let view = self.view(&share.vault)?;
        let mut slot = view.placed.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((generation, placed)) = slot.as_ref() {
            if *generation == current.generation {
                return placed.clone();
            }
        }
        // A note the vault could not read may place files: unknown, while
        // there are rules. With none, a note only places its own
        // property-attached files at the reference (#668), and one that
        // does not read attaches none: the notes that read are placed, so
        // one unreadable note never holds back every file of a default
        // link.
        let placed = if view.unread.is_empty() || current.is_empty() {
            let notes = view.notes.get_or_init(|| listing::every_note(&view.vault));
            LayerRules::place(&current, notes).map(Arc::new)
        } else {
            tracing::warn!(vault = %share.vault, unread = ?view.unread, "part of a shared vault does not read: share links below Full hold back every attached file");
            None
        };
        *slot = Some((current.generation, placed.clone()));
        placed
    }

    /// The files of `placed` by their folded path, for [`Self::above`].
    /// Built once per placement: the slot remembers which placement it
    /// folded (the same allocation), so a vault or rules change between
    /// [`Self::placed`] and here never answers from another placement's
    /// map. Blocking.
    fn folded(
        &self,
        share: &Share,
        placed: &Arc<HashMap<String, Listen>>,
    ) -> Arc<HashMap<String, Vec<String>>> {
        let fold = || {
            let mut folded: HashMap<String, Vec<String>> = HashMap::new();
            for file in placed.keys() {
                folded
                    .entry(fold_path(file))
                    .or_default()
                    .push(file.clone());
            }
            Arc::new(folded)
        };
        // No view to remember it on: folded for this request alone.
        let Some(view) = self.view(&share.vault) else {
            return fold();
        };
        let mut slot = view.folded.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((of, folded)) = slot.as_ref() {
            if Arc::ptr_eq(of, placed) {
                return Arc::clone(folded);
            }
        }
        let folded = fold();
        *slot = Some((Arc::clone(placed), Arc::clone(&folded)));
        folded
    }

    /// The attachments a link of `share` that plays to `cap` holds back:
    /// built but never named (`architect_sites::Options::hold_back`).
    /// Every attached file while placement is unknown; None while the
    /// vault cannot be read at all (build nothing then). Blocking.
    fn held_back(&self, share: &Share, cap: Listen) -> Option<BTreeSet<String>> {
        if cap == Listen::Full {
            return Some(BTreeSet::new());
        }
        let view = self.view(&share.vault)?;
        Some(self.placed(share).map_or_else(
            || {
                view.notes
                    .get_or_init(|| listing::every_note(&view.vault))
                    .iter()
                    .flat_map(|e| e.files.iter().cloned())
                    .collect()
            },
            |placed| {
                placed
                    .iter()
                    .filter(|(_, layer)| **layer > cap)
                    .map(|(file, _)| file.clone())
                    .collect()
            },
        ))
    }

    /// Whether the file at `rest` (a path below the link of `share` for
    /// `audience`, built into `dir`) is held back from a link that plays
    /// to `cap`. Blocking.
    ///
    /// Decided first by its key, from the placement (cached), among the
    /// files of this link alone (so a link says nothing of a file outside
    /// it), before the disk is touched. A held-back file is never built
    /// into a link, so only a folder from before a change of layer can
    /// hold one: then by what the request opens on disk, so no other
    /// spelling (its case, its Unicode form, `//`) reaches it. A folder or
    /// a file that cannot be read is held back ([`Hold::Building`]), never
    /// served. While placement is unknown every file of the link is held
    /// back.
    fn above(
        &self,
        share: &Share,
        audience: Option<&ShareAudience>,
        dir: &Path,
        rest: &str,
        cap: Listen,
    ) -> Hold {
        if cap == Listen::Full {
            return Hold::Serve;
        }
        // Where files are must be known, from a vault read whole: else a
        // link below Full answers nothing (a folder built before a change
        // may hold anything).
        if self.view(&share.vault).is_none() {
            return Hold::Building;
        }
        let Some(placed) = self.placed(share) else {
            return Hold::Building;
        };
        let decoded = decode(rest);
        let unsafe_path = decoded
            .split('/')
            .any(|s| s == ".." || s == "." || s.contains('\\') || s.contains('\0'));
        if unsafe_path {
            return Hold::Serve;
        }
        let key = decoded.trim_start_matches('/');
        let listing = self.listing(share, audience);
        let mine: std::collections::HashSet<&str> = listing
            .iter()
            .flat_map(|e| e.files.iter().map(String::as_str))
            .collect();
        // Above the link's layer: 403 if the link lists it, else the same
        // 404 as a path that is nowhere (it says nothing of a file it does
        // not list, even one an older build left in its folder).
        let answer = |file: &str, layer: Listen| {
            if mine.contains(file) {
                Hold::Held(layer)
            } else {
                Hold::Hidden
            }
        };
        // By key, before the disk.
        if let Some(layer) = placed.get(key).filter(|layer| **layer > cap) {
            return answer(key, *layer);
        }
        // On disk: what the folder holds now. Another spelling of a placed
        // file (its case, its Unicode form, `//`) is found by its folded
        // path and confirmed by what the request opens.
        if !dir.is_dir() {
            return Hold::Building;
        }
        let mut path = dir.join(key);
        if decoded.ends_with('/') || path.is_dir() {
            path = path.join("index.html");
        }
        match std::fs::metadata(&path) {
            Ok(asked) if asked.is_file() => {
                let folded = self.folded(share, &placed);
                folded
                    .get(&fold_path(key))
                    .into_iter()
                    .flatten()
                    .filter_map(|file| placed.get(file).filter(|l| **l > cap).map(|l| (file, *l)))
                    .find(|(file, _)| same_file(&asked, &dir.join(file.as_str())))
                    .map_or(Hold::Serve, |(file, layer)| answer(file, layer))
            }
            Ok(_) => Hold::Serve,
            // Not readable in a folder that is there: `file` reads the
            // same path and fails the same way (its 404).
            Err(_) if dir.is_dir() => Hold::Serve,
            // The folder went away under the request: held back.
            Err(_) => Hold::Building,
        }
    }

    /// Whether `audience`'s link answers `rest` (a path below it), in
    /// the vault as it is now. Blocking ([`Self::view`]).
    fn guard(&self, share: &Share, audience: &ShareAudience, rest: &str) -> Guard {
        let manifest = |code: &str| {
            let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
            let found = table.get(&share.code)?.manifests.get(code).cloned();
            drop(table);
            found
        };
        let (own, whole) = (manifest(&audience.code), manifest(&share.code));
        let Some(own) = own else {
            return Guard::Building;
        };
        let decoded = decode(rest);
        // A page's own file (its HTML) is the page: checked as its note.
        let key = decoded
            .strip_suffix("index.html")
            .filter(|dir| dir.ends_with('/'))
            .unwrap_or(&decoded);
        let dir = format!("{}/", key.trim_end_matches('/'));
        let note = |m: &Manifest| m.pages.get(key).or_else(|| m.pages.get(&dir)).cloned();
        let listed = || self.listing(share, Some(audience));
        if let Some(note) = note(&own) {
            return if listed().iter().any(|e| e.path == note) {
                Guard::Open
            } else {
                Guard::Refused
            };
        }
        if let Some(file) = own.files.get(key) {
            return if listed().iter().any(|e| e.files.contains(file)) {
                Guard::Open
            } else {
                Guard::Refused
            };
        }
        // A file of its own notes that its build held back (above its
        // layer, #528): its own, answered by the layer check after this.
        let vault_path = key.trim_start_matches('/');
        if listed()
            .iter()
            .any(|e| e.files.iter().any(|f| f == vault_path))
        {
            return Guard::Open;
        }
        // A note or a file of the share that is not this audience's.
        let others = whole.is_some_and(|w| note(&w).is_some() || w.files.contains_key(key));
        if others {
            Guard::Refused
        } else if own.pages.is_empty() {
            Guard::Empty
        } else {
            // A page of the audience's own site that is no note's: its
            // tags, its assets, its 404.
            Guard::Open
        }
    }

    /// Keep `vault`'s shares read and built: now, then whenever the vault
    /// changes or a share of it is made or changed here.
    pub async fn keep_built(self, vault: String) {
        let channel = self.0.backend.channel(&vault).await;
        let mut changes = channel.subscribe();
        // Heard apart from the rebuilds, which take a while: a request
        // that comes after a change is checked against the vault read
        // again, built or not.
        let watch = {
            let (moved, mut heard) = (self.moved(&vault), channel.subscribe());
            tokio::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match heard.recv().await {
                        Ok(_) | Err(RecvError::Lagged(_)) => {
                            moved.fetch_add(1, Ordering::SeqCst);
                        }
                        Err(RecvError::Closed) => return,
                    }
                }
            })
        };
        let _stop = AbortOnDrop(watch);
        let kick = self.kick(&vault);
        let pass = {
            let (me, vault) = (self.clone(), vault.clone());
            move || {
                me.load(&vault);
                me.build_all(&vault);
            }
        };
        // The layer rules' generation the pages were built with: a rules
        // change (an extension installed, fixed, removed, forgotten)
        // builds them again, so no page names a file its link holds back.
        let rules = {
            let layers = self.0.layers.clone();
            move || {
                let layers = layers.clone();
                async move {
                    tokio::task::spawn_blocking(move || layers.current().generation)
                        .await
                        .unwrap_or_default()
                }
            }
        };
        let mut built_with = rules().await;
        let _ = tokio::task::spawn_blocking(pass.clone()).await;
        let mut every = tokio::time::interval(RULES_EVERY);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                heard = changes.recv() => {
                    if matches!(heard, Err(tokio::sync::broadcast::error::RecvError::Closed)) {
                        return;
                    }
                    tokio::time::sleep(SETTLE).await;
                    while changes.try_recv().is_ok() {}
                }
                () = kick.notified() => {}
                _ = every.tick() => {
                    if rules().await == built_with {
                        continue;
                    }
                }
            }
            built_with = rules().await;
            let _ = tokio::task::spawn_blocking(pass.clone()).await;
        }
    }

    /// Write `share`'s note and hold it now (so the next request sees
    /// it), then have its vault's shares rebuilt.
    async fn save(&self, share: Share) -> Result<Share, ShareFault> {
        let backend = self.0.backend.clone();
        let (vault, path, text) = (
            share.vault.clone(),
            share_note_path(&share.code),
            share_note(&share),
        );
        put(backend, vault, path, text).await?;
        self.moved(&share.vault).fetch_add(1, Ordering::SeqCst);
        {
            let mut table = self.0.table.write().unwrap_or_else(PoisonError::into_inner);
            match table.get_mut(&share.code) {
                Some(held) => held.share = share.clone(),
                None => {
                    table.insert(share.code.clone(), Held::new(share.clone()));
                }
            }
        }
        if share.state(now()) != ShareState::Live {
            let _ = std::fs::remove_dir_all(self.0.out.join(&share.code));
            // The pages' event stream fires: an open page reloads to 404.
            let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
            if let Some(held) = table.get(&share.code) {
                held.built.send_modify(|v| *v = v.wrapping_add(1));
            }
        }
        self.kick(&share.vault).notify_one();
        Ok(share)
    }

    /// Make a share for `by`.
    ///
    /// # Errors
    ///
    /// An invalid share, a vault this node does not hold, or its note
    /// could not be written.
    pub async fn create(&self, mut new: NewShare, by: &str) -> Result<Share, ShareFault> {
        let at = now();
        // No vault named: this node's only one.
        if new.vault.trim().is_empty() {
            if let [only] = self.0.backend.vault_ids().as_slice() {
                new.vault.clone_from(only);
            }
        }
        validate(&new, at)?;
        if self.0.backend.root(&new.vault).is_err() {
            return Err(ShareFault::Invalid(format!(
                "this node holds no vault `{}`",
                new.vault
            )));
        }
        self.save(Share {
            code: self.new_code(),
            vault: new.vault,
            scope: new.scope,
            title: new.title.filter(|t| !t.trim().is_empty()),
            audience: new.audience.normalised(),
            access: new.access,
            expires_at: new.expires_at,
            revoked_at: None,
            created_at: at,
            created_by: by.to_owned(),
            domain: new.domain.map(|d| d.trim().to_lowercase()),
            audiences: Vec::new(),
            listen: new.listen,
        })
        .await
    }

    /// Change share `code`.
    ///
    /// # Errors
    ///
    /// No such share, a revoked one, or an invalid change.
    pub async fn update(&self, code: &str, change: ShareChange) -> Result<Share, ShareFault> {
        let mut share = self
            .get(code)
            .ok_or_else(|| ShareFault::NotFound(code.to_owned()))?;
        if share.revoked_at.is_some() {
            return Err(ShareFault::Invalid(
                "a revoked share stays revoked".to_owned(),
            ));
        }
        match change {
            ShareChange::Audience(a) => share.audience = a.normalised(),
            ShareChange::Access(a) => share.access = a,
            ShareChange::Listen(l) => share.listen = l,
            ShareChange::Expires(at) => share.expires_at = at,
            ShareChange::Title(t) => share.title = t.filter(|t| !t.trim().is_empty()),
            ShareChange::Domain(d) => {
                let d = d.map(|d| d.trim().to_lowercase()).filter(|d| !d.is_empty());
                if let Some(d) = &d {
                    validate_domain(d)?;
                    let taken = self
                        .list()
                        .iter()
                        .any(|s| s.code != share.code && s.domain.as_deref() == Some(d));
                    if taken {
                        return Err(ShareFault::Invalid(format!("another share has {d}")));
                    }
                }
                share.domain = d;
            }
        }
        self.save(share).await
    }

    /// Revoke share `code`: from now on its link is `404`.
    ///
    /// # Errors
    ///
    /// No such share, or its note could not be written.
    pub async fn revoke(&self, code: &str) -> Result<Share, ShareFault> {
        let mut share = self
            .get(code)
            .ok_or_else(|| ShareFault::NotFound(code.to_owned()))?;
        if share.revoked_at.is_none() {
            share.revoked_at = Some(now());
        }
        self.save(share).await
    }

    /// A code no share and no audience has.
    fn new_code(&self) -> String {
        loop {
            let random = uuid::Uuid::new_v4();
            let mut bytes = [0_u8; 10];
            bytes.copy_from_slice(random.as_bytes().get(..10).unwrap_or(&[0; 10]));
            let code = code_from(bytes);
            if self.link(&code).is_none() {
                return code;
            }
        }
    }

    /// The share a link's code opens, and its audience when the code is
    /// one audience's (#510); a share's own code is the whole scope.
    #[must_use]
    pub fn link(&self, code: &str) -> Option<(Share, Option<ShareAudience>)> {
        let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = table.get(code) {
            return Some((held.share.clone(), None));
        }
        table.values().find_map(|held| {
            let audience = held.share.audience_at(code)?.clone();
            Some((held.share.clone(), Some(audience)))
        })
    }

    /// Add a named audience to share `code` (#510): a filter over its
    /// scope, on a link of its own.
    ///
    /// # Errors
    ///
    /// No such share, a share that is not live, a name it already has,
    /// or a filter that is not a Bases filter expression.
    pub async fn add_audience(&self, code: &str, new: NewAudience) -> Result<Share, ShareFault> {
        let mut share = self
            .get(code)
            .ok_or_else(|| ShareFault::NotFound(code.to_owned()))?;
        if share.state(now()) != ShareState::Live {
            return Err(ShareFault::Invalid(
                "only a live share takes an audience".to_owned(),
            ));
        }
        validate_audience(&share, &new)?;
        let filter = new.filter.trim().to_owned();
        architect_sites::listing::parse_filter(&filter)
            .map_err(|why| ShareFault::Invalid(format!("the filter `{filter}`: {why}")))?;
        share.audiences.push(ShareAudience {
            name: new.name.trim().to_owned(),
            filter,
            code: self.new_code(),
            created_at: now(),
            listen: new.listen,
        });
        self.save(share).await
    }

    /// How far audience `name`'s link of share `code` plays: `listen`,
    /// never past the share's; None: the share's.
    ///
    /// # Errors
    ///
    /// No such share or audience, a revoked share, or its note could not
    /// be written.
    pub async fn audience_listen(
        &self,
        code: &str,
        name: &str,
        listen: Option<Listen>,
    ) -> Result<Share, ShareFault> {
        let mut share = self
            .get(code)
            .ok_or_else(|| ShareFault::NotFound(code.to_owned()))?;
        if share.revoked_at.is_some() {
            return Err(ShareFault::Invalid(
                "a revoked share stays revoked".to_owned(),
            ));
        }
        let Some(audience) = share
            .audiences
            .iter_mut()
            .find(|a| a.name.trim().eq_ignore_ascii_case(name.trim()))
        else {
            return Err(ShareFault::NotFound(format!("{code}: audience {name}")));
        };
        audience.listen = listen;
        self.save(share).await
    }

    /// Remove audience `name` from share `code`: its link is `404` from
    /// now on, the share's own link and its other audiences stay.
    ///
    /// # Errors
    ///
    /// No such share or audience, or its note could not be written.
    pub async fn remove_audience(&self, code: &str, name: &str) -> Result<Share, ShareFault> {
        let mut share = self
            .get(code)
            .ok_or_else(|| ShareFault::NotFound(code.to_owned()))?;
        let Some(gone) = share.audience_named(name).cloned() else {
            return Err(ShareFault::NotFound(format!("{code}: audience {name}")));
        };
        share.audiences.retain(|a| a.code != gone.code);
        let _ = std::fs::remove_dir_all(self.audience_dir(&share.code, &gone.code));
        self.save(share).await
    }

    /// Have every vault's shares built again (the layer rules changed).
    fn kick_all(&self) {
        for vault in self.0.backend.vault_ids() {
            self.kick(&vault).notify_one();
        }
    }

    /// Where an audience's pages are built.
    fn audience_dir(&self, share: &str, audience: &str) -> PathBuf {
        self.0.out.join(share).join("audiences").join(audience)
    }

    /// The comments on share `code`, oldest first.
    #[must_use]
    pub fn comments(&self, code: &str) -> Vec<ShareComment> {
        let Some(share) = self.get(code) else {
            return Vec::new();
        };
        self.0
            .backend
            .get_file(&share.vault, &comments_note_path(code))
            .map(|b| parse_comments(&String::from_utf8_lossy(&b.0)))
            .unwrap_or_default()
    }

    /// The share served at `host` (a domain of its own), if any.
    fn at_host(&self, host: &str) -> Option<String> {
        let host = host.split(':').next().unwrap_or_default().to_lowercase();
        let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
        table
            .values()
            .find(|h| h.share.domain.as_deref() == Some(host.as_str()))
            .map(|h| h.share.code.clone())
    }

    /// Who holds issuer session `token`, resolved by the issuer (cached
    /// a minute): the bearer or the `architect_session` cookie of a
    /// page's request, a guest's `token`. None: nobody signed in.
    async fn viewer_of(&self, token: Option<String>) -> Viewer {
        let Some(token) = token.filter(|t| !t.trim().is_empty()) else {
            return Viewer::Anonymous;
        };
        {
            let seen = self
                .0
                .viewers
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if let Some((who, at)) = seen.get(&token) {
                if at.elapsed() < VIEWER_TTL {
                    return who.clone();
                }
            }
        }
        let Some(issuer) = &self.0.issuer else {
            return Viewer::Anonymous;
        };
        // Every issuer call is bounded (`IssuerLane::within`, which also
        // drops a stalled connection): an issuer that never answers makes
        // the viewer anonymous promptly instead of hanging the page.
        let user = issuer
            .within(async {
                match issuer.client::<auth_proto::AuthServiceClient>().await {
                    Ok(auth) => auth.whoami(token.clone()).await.map_err(|e| e.to_string()),
                    Err(e) => Err(e),
                }
            })
            .await
            .unwrap_or_else(|| Err(ISSUER_SILENT.to_owned()));
        let user = match user {
            Ok(user) => user,
            Err(why) => {
                tracing::debug!(%why, "a share viewer's session the issuer did not resolve");
                if why.contains("connect") || why.contains("closed") {
                    issuer.evict();
                }
                return Viewer::Anonymous;
            }
        };
        let member = match &self.0.org {
            Some(org) => {
                issuer
                    .within(async {
                        match issuer
                            .client::<auth_proto::OrganizationServiceClient>()
                            .await
                        {
                            Ok(client) => IdentityServer(client)
                                .orgs(&token)
                                .await
                                .is_ok_and(|orgs| orgs.iter().any(|o| o.id == org.id)),
                            Err(_) => false,
                        }
                    })
                    .await
            }
            None => Some(false),
        };
        // No answer: not a member this time, and not remembered as one
        // who is not.
        let Some(member) = member else {
            return Viewer::Person {
                user_id: user.id.to_string(),
                email: user.email.map(|e| e.to_lowercase()),
                member: false,
            };
        };
        let who = Viewer::Person {
            user_id: user.id.to_string(),
            email: user.email.map(|e| e.to_lowercase()),
            member,
        };
        let mut seen = self
            .0
            .viewers
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        seen.retain(|_, (_, at)| at.elapsed() < VIEWER_TTL);
        seen.insert(token, (who.clone(), Instant::now()));
        who
    }
}

/// A task stopped when this is dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Write `text` to `path` in `vault`, off the runtime's workers (the
/// write announces itself under a blocking lock).
async fn put(
    backend: Backend,
    vault: String,
    path: String,
    text: String,
) -> Result<(), ShareFault> {
    tokio::task::spawn_blocking(move || {
        backend.register_collab(&vault, &path)?;
        backend.put_file(&vault, &path, text.into_bytes(), IfMatch::Force)
    })
    .await
    .map_err(|e| ShareFault::Internal(e.to_string()))?
    .map(|_| ())
    .map_err(|e| ShareFault::Internal(e.to_string()))
}

/// A share's scope as Sites builds it.
fn scope(scope: &ShareScope) -> architect_sites::Scope {
    match scope {
        ShareScope::Vault => architect_sites::Scope::Vault,
        ShareScope::Folder(f) => architect_sites::Scope::Folder(f.clone()),
        ShareScope::Tag(t) => architect_sites::Scope::Tag(t.clone()),
        ShareScope::Notes(n) => architect_sites::Scope::Notes(n.clone()),
        ShareScope::Base(b) => architect_sites::Scope::Base(b.clone()),
    }
}

/// [`ShareLinks`] for the caller the org lane's gate resolved.
#[derive(Clone)]
pub struct SharesLane(pub Shares);

fn caller() -> Result<String, ShareFault> {
    match architect::permissions_gate::caller() {
        Some(Principal::User { user_id }) => Ok(user_id),
        _ => Err(ShareFault::Unauthenticated),
    }
}

impl ShareLinks for SharesLane {
    async fn create(&self, new: NewShare) -> Result<Share, ShareFault> {
        let by = caller()?;
        self.0.create(new, &by).await
    }

    async fn shares(&self) -> Result<Vec<Share>, ShareFault> {
        caller()?;
        Ok(self.0.list())
    }

    async fn update(&self, code: String, change: ShareChange) -> Result<Share, ShareFault> {
        caller()?;
        self.0.update(&code, change).await
    }

    async fn revoke(&self, code: String) -> Result<Share, ShareFault> {
        caller()?;
        self.0.revoke(&code).await
    }

    async fn add_audience(&self, code: String, new: NewAudience) -> Result<Share, ShareFault> {
        caller()?;
        self.0.add_audience(&code, new).await
    }

    async fn remove_audience(&self, code: String, name: String) -> Result<Share, ShareFault> {
        caller()?;
        self.0.remove_audience(&code, &name).await
    }

    async fn audience_listen(
        &self,
        code: String,
        name: String,
        listen: Option<Listen>,
    ) -> Result<Share, ShareFault> {
        caller()?;
        self.0.audience_listen(&code, &name, listen).await
    }

    async fn kept_layers(&self) -> Result<LayersReport, ShareFault> {
        caller()?;
        let layers = self.0.0.layers.clone();
        blocking(move || layers.report())
            .await
            .ok_or_else(|| ShareFault::Internal("the kept rules did not read".to_owned()))
    }

    async fn forget_layers(&self, extension: String) -> Result<LayersReport, ShareFault> {
        caller()?;
        let layers = self.0.0.layers.clone();
        let forgotten = blocking(move || layers.forget(&extension))
            .await
            .ok_or_else(|| ShareFault::Internal("the kept rules did not read".to_owned()))?
            .map_err(ShareFault::Invalid)?;
        self.0.kick_all();
        Ok(forgotten)
    }

    async fn acknowledge_layers(&self) -> Result<LayersReport, ShareFault> {
        caller()?;
        let layers = self.0.0.layers.clone();
        let report = blocking(move || layers.acknowledge())
            .await
            .ok_or_else(|| ShareFault::Internal("the kept rules did not read".to_owned()))?;
        self.0.kick_all();
        Ok(report)
    }

    async fn comments(&self, code: String) -> Result<Vec<ShareComment>, ShareFault> {
        caller()?;
        if self.0.get(&code).is_none() {
            return Err(ShareFault::NotFound(code));
        }
        Ok(self.0.comments(&code))
    }
}

// ── HTTP ─────────────────────────────────────────────────────────────

/// The node's outermost layer: share links, their sign-in and domains.
///
/// `/s/<code>/…` and a share's own domain are answered here, `/s/_signin…`
/// and `/s/_signout` are sign-in; everything else goes on (an unknown
/// `/s/…` is `404` here, never the node's other routes). Every answer
/// given here carries [`content_security_policy`].
pub async fn gate(State(shares): State<Shares>, request: Request, next: Next) -> Response {
    // The node's own `/_ext/` (studio#445), on any host a share has too:
    // the islands' files, their runtime and their sandbox document, each
    // with its own headers and never the share's policy (two CSPs would
    // both hold, and the share's would stop the sandbox's script).
    if request
        .uri()
        .path()
        .starts_with(architect_sites::island::ISLANDS_PATH)
    {
        // Files: read, never written to.
        if request.method() != Method::GET && request.method() != Method::HEAD {
            let mut response = page(
                StatusCode::METHOD_NOT_ALLOWED,
                "Not allowed",
                "<p>Only GET and HEAD.</p>",
            );
            let headers = response.headers_mut();
            headers.insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
            headers.insert(header::CONTENT_SECURITY_POLICY, content_security_policy());
            headers.insert(
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            );
            return response;
        }
        return islands(&shares, request.uri().path());
    }
    let guest = is_guest_path(request.uri().path());
    // A web app on another origin calls a link's guest service too: the
    // code is the credential and a session travels in the body, so any
    // origin may, without credentials: `Access-Control-Allow-Origin: *`
    // and never `-Allow-Credentials`, so a browser never sends this node's
    // cookie on another origin's call (nor lets it read the answer of one
    // that carried it). The node's own sign-in cookie stands in for an
    // empty token only on a same-site call, the app on this origin
    // (`SameSite=Lax`, below).
    if guest && request.method() == Method::OPTIONS {
        let mut response = StatusCode::NO_CONTENT.into_response();
        let headers = response.headers_mut();
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_METHODS,
            HeaderValue::from_static("POST"),
        );
        headers.insert(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            HeaderValue::from_static("content-type, authorization"),
        );
        headers.insert(
            header::ACCESS_CONTROL_MAX_AGE,
            HeaderValue::from_static("600"),
        );
        return guest_headers(response);
    }
    match answer(shares, request).await {
        Ok(response) => with_policy(response),
        // A link's guest service (`ShareGuest`, mounted on the node's
        // router): its answers as private as the link's own.
        // The node's own sign-in cookie, from the app on this origin (the
        // browser sends it only to a same-site request: `SameSite=Lax`),
        // stands in for a token the call leaves empty ([`GuestLane`]).
        Err(request) if guest => {
            let session = cookie(request.headers(), SESSION_COOKIE);
            guest_headers(GUEST_COOKIE.scope(session, next.run(*request)).await)
        }
        Err(request) => next.run(*request).await,
    }
}

/// The path of a link's guest service: `/s/<code>/_guest/…`.
fn is_guest_path(path: &str) -> bool {
    path.strip_prefix(LINKS)
        .and_then(|r| r.strip_prefix('/'))
        .and_then(|r| r.split_once('/'))
        .is_some_and(|(code, rest)| is_code(code) && rest.starts_with(GUEST_SEGMENT))
}

tokio::task_local! {
    /// A guest call's `architect_session` cookie, set by [`gate`] around
    /// the call.
    static GUEST_COOKIE: Option<String>;
}

/// The segment below a link its guest service is at.
const GUEST_SEGMENT: &str = "_guest/";

/// What every answer of a guest service carries: no referrer, not
/// stored, not indexed, not sniffed.
fn guest_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        "x-robots-tag",
        HeaderValue::from_static("noindex, nofollow"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    response
}

/// Where the web app (`/app/`, #495) reads the node's islands: what a
/// share page carries embedded (`architect_sites::island::page_json`),
/// the same for everyone who asks.
pub const ISLANDS_LISTING: &str = "/_ext/islands.json";

/// [`ISLANDS_LISTING`]: the islands, their folders, views and each web
/// file's digest, as JSON. Never cached (`no-cache`): the paths it names
/// are content-addressed and change with a new island, the listing keeps
/// its name. The app pins each file it fetches by the digest it reads
/// here; nothing in it is a secret.
fn islands_listing(shares: &Shares) -> Response {
    let body = architect_sites::island::page_json(shares.islands()).to_string();
    let mut response = Response::new(Body::from(body));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    // Data, never a page: the share's policy, should one be opened.
    headers.insert(header::CONTENT_SECURITY_POLICY, content_security_policy());
    response
}

/// Where the web app (`/app/`, #607) reads the note types' favourites:
/// the distribution's extensions' `[[favourites]]`, the same for everyone
/// who asks.
pub const FAVOURITES_LISTING: &str = "/_ext/favourites.json";

/// [`FAVOURITES_LISTING`]: every `[[favourites]]` declaration that keeps
/// the manifest's rules (`TypeFavourites::check`), each with the
/// extension that declared it and what its capabilities let it run
/// (`architect_favourites::Listed`: the app drops a target it may not
/// name), as JSON (`{"favourites": [{"by": …, "favourites": …}]}`), and
/// nothing else: no vault, no share, no extension's other parts. Never
/// cached (`no-cache`): a new release declares again under the same name.
fn favourites_listing(shares: &Shares) -> Response {
    let list: Vec<&architect_favourites::Listed> = shares
        .listed_favourites()
        .iter()
        .filter(|f| f.favourites.check().is_ok())
        .collect();
    let body = serde_json::json!({ "favourites": list }).to_string();
    let mut response = Response::new(Body::from(body));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::CONTENT_SECURITY_POLICY, content_security_policy());
    response
}

/// What the node answers under `/_ext/`: an island's file, the runtime or
/// the sandbox document (`architect_sites::island::serve`), `404` for
/// anything else. Every answer is `nosniff` and cached for good (each
/// path is named by its content); the sandbox document carries its own
/// policy (`sandbox_headers`), and a `.js` or `.wasm` the share's.
fn islands(shares: &Shares, path: &str) -> Response {
    if path == ISLANDS_LISTING {
        return islands_listing(shares);
    }
    if path == FAVOURITES_LISTING {
        return favourites_listing(shares);
    }
    let Some((body, kind)) = architect_sites::island::serve(shares.islands(), path) else {
        let mut response = page(StatusCode::NOT_FOUND, "Not found", "<p>No such file.</p>");
        let headers = response.headers_mut();
        headers.insert(header::CONTENT_SECURITY_POLICY, content_security_policy());
        headers.insert(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        );
        return response;
    };
    let mut response = Response::new(Body::from(body.into_owned()));
    let headers = response.headers_mut();
    if let Some(own) = architect_sites::island::document_headers(path) {
        // A frame's document (the sandbox, a web view): its own policy,
        // every header of it, or no document at all: served without its
        // policy it would run with this origin's.
        for (name, value) in own {
            let (Ok(header_name), Ok(header_value)) = (
                header::HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(&value),
            ) else {
                tracing::error!(
                    header = name,
                    "the island sandbox's header does not read: no sandbox served"
                );
                return page(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Not available",
                    "<p>No such file.</p>",
                );
            };
            headers.insert(header_name, header_value);
        }
        return response;
    }
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(kind));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    // A module or a component is fetched, never a page: the share's
    // policy, should one be opened.
    headers.insert(header::CONTENT_SECURITY_POLICY, content_security_policy());
    response
}

/// `/.well-known/apple-app-site-association` (#629), from the node's
/// configuration (`apple_app_ids`).
///
/// It names the iOS apps that open this node's share links (`/s/*`,
/// never its sign-in `/s/_*` or a link's guest service) as universal
/// links. None configured: not served (None).
#[must_use]
pub fn apple_app_site_association(app_ids: &[String]) -> Option<String> {
    if app_ids.is_empty() {
        return None;
    }
    let body = serde_json::json!({
        "applinks": {
            "details": [{
                "appIDs": app_ids,
                // First match wins; `*` is any run of characters, `/`
                // too. Only a link's pages are the app's: every name the
                // node reserves (`_archive.zip`, `_files/`, `_files.json`,
                // `_guest/`, `_live`, `_note.json`) and every file (media,
                // a PDF, an image: no trailing `/`) stays the browser's, so
                // "Download all" downloads (#676).
                "components": [
                    { "/": format!("{LINKS}/_*"), "exclude": true, "comment": "the node's sign-in" },
                    { "/": format!("{LINKS}/*/_*"), "exclude": true, "comment": "a link's reserved names: its guest service, files, archive, news" },
                    { "/": "/a/*", "comment": "deep links into the app's actions (#186): architect_app::links" },
                    { "/": format!("{LINKS}/*/"), "comment": "a share link's pages" }
                ]
            }]
        }
    });
    Some(body.to_string())
}

/// The Content-Security-Policy of every page of a share or a Site the node
/// serves (#520's review).
///
/// It is the second line of defence behind `architect_sites::sanitize`.
/// Scripts and everything else from the node
/// alone, plus the site's one inline script by its hash
/// (`architect_sites::ui::THEME_BOOT_HASH`); images also `data:` and
/// `https:`; no plugins, no `<base>`, forms posted only to the node, no
/// framing by other sites. Inline styles stay allowed (a block's SVG
/// sheet, the share pages' own); with `default-src 'self'` no style can
/// load or send anything off the node.
pub fn content_security_policy() -> HeaderValue {
    // Load-bearing for an island's web view frame: `default-src 'self'`
    // stands in for `frame-src`, so a frame on a share page (the island's
    // sandbox, a web view, `architect_sites::island`) can only ever be the
    // node's own: a web view navigating itself elsewhere is refused here,
    // not by its own policy (a frame's CSP cannot stop its own navigation).
    // Widen `frame-src` and a web view's bundle can leave the node.
    static POLICY: std::sync::OnceLock<HeaderValue> = std::sync::OnceLock::new();
    POLICY
        .get_or_init(|| {
            let policy = format!(
                "default-src 'self'; script-src 'self' {}; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'self'; img-src 'self' data: https:; media-src 'self'; style-src 'self' 'unsafe-inline'",
                architect_sites::ui::THEME_BOOT_HASH
            );
            HeaderValue::from_str(&policy).unwrap_or_else(|_| HeaderValue::from_static("default-src 'self'"))
        })
        .clone()
}

/// `response` with [`content_security_policy`] and `nosniff`; an answer
/// that has a policy of its own (the web app's page, `share_shell`) keeps
/// it.
fn with_policy(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers
        .entry(header::CONTENT_SECURITY_POLICY)
        .or_insert_with(content_security_policy);
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

/// A node's Sites (`Config::sites`, mounted at `mounts`): every answer
/// under a mount carries [`content_security_policy`], as a share's does.
pub async fn sites_policy(
    State(mounts): State<Arc<Vec<String>>>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let under = mounts.iter().any(|m| {
        let m = m.trim_end_matches('/');
        m.is_empty()
            || path
                .strip_prefix(m)
                .is_some_and(|r| r.is_empty() || r.starts_with('/'))
    });
    let response = next.run(request).await;
    if under {
        with_policy(response)
    } else {
        response
    }
}

/// What [`gate`] answers; the request back when it is not a share's.
async fn answer(shares: Shares, request: Request) -> Result<Response, Box<Request>> {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if let Some(share) = shares.at_host(&host).and_then(|code| shares.get(&code)) {
        let rest = request.uri().path().to_owned();
        return Ok(serve(&shares, share, None, &rest, "/", request).await);
    }
    let path = request.uri().path().to_owned();
    let Some(below) = path
        .strip_prefix(LINKS)
        .filter(|r| r.is_empty() || r.starts_with('/'))
    else {
        return Err(Box::new(request));
    };
    let below = below.trim_start_matches('/');
    if let Some(rest) = below.strip_prefix('_') {
        return Ok(signin(&shares, rest, request).await);
    }
    // A link's guest service: the router's (it opens the link itself).
    if is_guest_path(&path) {
        return Err(Box::new(request));
    }
    let mut parts = below.splitn(2, '/');
    let first = parts.next().unwrap_or_default();
    // Any code to its folder, known or not: a link that is nowhere, an
    // expired or revoked one and one that never was answer alike.
    if is_code(first) && !below.contains('/') {
        return Ok(Redirect::permanent(&link_path(first)).into_response());
    }
    let Some((share, audience)) = Some(first)
        .filter(|code| is_code(code))
        .and_then(|code| shares.link(code))
    else {
        return Ok(gone());
    };
    let mount = link_path(first);
    let rest = format!("/{}", parts.next().unwrap_or_default());
    Ok(serve(&shares, share, audience, &rest, &mount, request).await)
}

/// `work` on the blocking pool (it reads a vault).
async fn blocking<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(work).await.ok()
}

/// Answer `rest` (a path below the link, from `/`) of `share`, or of
/// its `audience` when the link is one audience's; the link is at
/// `mount`.
///
/// A page path (`/`, anything ending in `/`, a built page's folder) is
/// the web app's page ([`share_shell`]), which opens the link as its
/// guest; the files page (`_files/`) too, where the app shows the
/// link's files. Every other path is a file of the link, its bytes. Both
/// go through the same two checks ([`Shares::open`], [`admit`]), so an
/// expired, revoked or refused link and a page that is nowhere answer as
/// before. On a share's own domain (`mount` `/`) the link is a Site
/// instead: its built pages, as `ARCHITECT_SITES` are served.
async fn serve(
    shares: &Shares,
    share: Share,
    audience: Option<ShareAudience>,
    rest: &str,
    mount: &str,
    request: Request,
) -> Response {
    let code = share.code.as_str();
    let domain = mount == "/";
    let issuer = shares.0.issuer_origin.as_deref();
    let web = shares.web_app();
    let head = |open: bool| share_shell::Head {
        title: open.then(|| {
            Link {
                share: &share,
                audience: audience.as_ref(),
                mount,
                listen: share.listen_for(audience.as_ref()),
            }
            .title()
        }),
        public: matches!(share.audience, architect_spaces::Audience::Public),
    };
    let page_path = !domain && is_page_path(rest);
    if let Err(closed) = shares
        .open(
            &share,
            bearer(request.headers()).or_else(|| cookie(request.headers(), SESSION_COOKIE)),
        )
        .await
    {
        return match closed {
            Closed::Gone => gone(),
            // The app asks the guest service, hears why, and offers the
            // sign-in (`/s/_signin`): the page names nothing of the link.
            Closed::SignIn if page_path => private(
                &share,
                share_shell::answer(StatusCode::UNAUTHORIZED, &head(false), issuer, web),
            ),
            Closed::Denied if page_path => private(
                &share,
                share_shell::answer(StatusCode::FORBIDDEN, &head(false), issuer, web),
            ),
            Closed::SignIn => plain(StatusCode::UNAUTHORIZED, "Sign in to open this link."),
            Closed::Denied => plain(
                StatusCode::FORBIDDEN,
                "This link was not shared with the account you are signed in as.",
            ),
        };
    }
    let base = if domain { "domain" } else { "site" };
    let dir = audience.as_ref().map_or_else(
        || shares.0.out.join(code).join(base),
        |a| shares.audience_dir(code, &a.code).join("site"),
    );
    let link = Link {
        share: &share,
        audience: audience.as_ref(),
        mount,
        listen: share.listen_for(audience.as_ref()),
    };
    let query = request.uri().query().unwrap_or_default().to_owned();
    let headers = request.headers().clone();
    match rest {
        "/_live" => live(shares, code),
        "/_files.json" => private(&share, files_json(shares, &link, &query).await),
        "/_archive.zip" => private(&share, archive(shares, &link, &query).await),
        // The files page is the app's Files view of the link.
        "/_files" | "/_files/" if !domain => private(
            &share,
            share_shell::answer(StatusCode::OK, &head(true), issuer, web),
        ),
        _ if page_path || (!domain && is_built_page(&dir, rest)) => {
            // Admitted as the page it opens is: a page another audience
            // holds, or none, is nowhere.
            let page = format!("{}/index.html", rest.trim_end_matches('/'));
            match admit(shares, &share, audience.as_ref(), &dir, &page, link.listen).await {
                Admit::Open if is_built_page(&dir, rest) => private(
                    &share,
                    share_shell::answer(StatusCode::OK, &head(true), issuer, web),
                ),
                Admit::Open | Admit::Hidden | Admit::Held(_) => private(&share, gone()),
                // The app hears the same from the guest service and
                // waits; the page is the app all the same.
                Admit::Building => private(&share, {
                    let mut r = share_shell::answer(
                        StatusCode::SERVICE_UNAVAILABLE,
                        &head(true),
                        issuer,
                        web,
                    );
                    r.headers_mut()
                        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
                    r
                }),
                Admit::Empty => private(
                    &share,
                    share_shell::answer(StatusCode::NOT_FOUND, &head(true), issuer, web),
                ),
            }
        }
        _ => {
            // A file: never a page the build wrote (its HTML is the
            // build's, read by nobody), but a domain's.
            if !domain && is_page_file(rest) {
                return private(&share, gone());
            }
            match admit(shares, &share, audience.as_ref(), &dir, rest, link.listen).await {
                Admit::Open => {}
                Admit::Hidden => {
                    return private(
                        &share,
                        if domain {
                            not_found(&dir).await
                        } else {
                            gone()
                        },
                    );
                }
                Admit::Held(needs) => return private(&share, layer_refusal(link.listen, needs)),
                Admit::Building => return private(&share, being_built()),
                Admit::Empty => return private(&share, gone()),
            }
            let response = file(&dir, rest, &headers, domain).await;
            // A link's own file never runs on this origin (a domain's Site
            // is its own origin's, and keeps its pages).
            let response = if domain { response } else { inert(response) };
            private(&share, response)
        }
    }
}

/// Whether `rest` (below a link, from `/`) names a page: the link's
/// root, or anything ending in `/`.
fn is_page_path(rest: &str) -> bool {
    rest.ends_with('/')
}

/// Whether `rest` is a folder the build wrote a page into (`/joy`
/// without its slash), its page a file of the link's own output.
fn is_built_page(dir: &Path, rest: &str) -> bool {
    let decoded = decode(rest);
    if decoded
        .split('/')
        .any(|s| s == ".." || s == "." || s.contains('\\') || s.contains('\0'))
    {
        return false;
    }
    let page = dir.join(decoded.trim_start_matches('/')).join("index.html");
    match (dir.canonicalize(), page.canonicalize()) {
        (Ok(root), Ok(resolved)) => resolved.starts_with(&root) && resolved.is_file(),
        _ => false,
    }
}

/// Whether `rest` names a page the build wrote (`index.html`, its
/// `404.html`): never served on a link.
fn is_page_file(rest: &str) -> bool {
    let decoded = decode(rest);
    let last = decoded.rsplit('/').next().unwrap_or_default();
    last.eq_ignore_ascii_case("index.html") || decoded.eq_ignore_ascii_case("/404.html")
}

/// `404`: nothing at this link (expired, revoked, unknown, or a path
/// that is nowhere on it: all one answer, its headers too: stored by
/// nobody, no referrer, not indexed).
fn gone() -> Response {
    let mut response = plain(StatusCode::NOT_FOUND, "There is nothing at this link.");
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(
        "x-robots-tag",
        HeaderValue::from_static("noindex, nofollow"),
    );
    response
}

/// What a browser may show of a link's own file, on this origin: a
/// picture, a recording, a video, plain text or a PDF.
fn shown_inline(kind: &str) -> bool {
    let base = kind
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(
        base.as_str(),
        "image/png"
            | "image/jpeg"
            | "image/gif"
            | "image/webp"
            | "image/avif"
            | "text/plain"
            | "application/pdf"
    ) || base.starts_with("audio/")
        || base.starts_with("video/")
}

/// A link's own file as it may leave this origin's node: anything a
/// browser could run (HTML, SVG, XML, XHTML, script, and any type not
/// [`shown_inline`]) is a download (`Content-Disposition: attachment`)
/// under a sandbox with nothing allowed, so a page a note links runs
/// nowhere near the web app's session on this origin. Pictures and
/// recordings stay inline (the app shows and plays them).
fn inert(mut response: Response) -> Response {
    let kind = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if !shown_inline(&kind) {
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(INERT_POLICY),
        );
        headers.insert(
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("attachment"),
        );
    }
    response
}

/// The policy on a link's file a browser could run ([`inert`]).
const INERT_POLICY: &str = "sandbox; default-src 'none'";

/// A plain-text answer of the node's own.
fn plain(status: StatusCode, text: &'static str) -> Response {
    let mut response = (status, text).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// Why a link is not open to a caller ([`Shares::open`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closed {
    /// Expired, revoked or unknown.
    Gone,
    /// For signed-in people, and nobody is.
    SignIn,
    /// Signed in, but not as anyone it is for.
    Denied,
}

/// Whether a path below a link is served ([`admit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Admit {
    Open,
    /// Answered as a path that is nowhere: another audience's note or
    /// file, or one above the layer the link does not list.
    Hidden,
    /// Above the link's layer: it needs this one.
    Held(Listen),
    /// Being built, or not readable now.
    Building,
    /// An audience nothing is in.
    Empty,
}

impl Shares {
    /// Whether `share` is open to the holder of issuer session `token`
    /// now: live, and its audience admits them. The first check of every
    /// request to a link, a page's or a guest's ([`GuestLane`]).
    async fn open(&self, share: &Share, token: Option<String>) -> Result<Viewer, Closed> {
        if share.state(now()) != ShareState::Live {
            return Err(Closed::Gone);
        }
        let viewer = if share.audience.signed_in() {
            self.viewer_of(token).await
        } else {
            Viewer::Anonymous
        };
        match share.admits(&viewer) {
            Verdict::Open => Ok(viewer),
            Verdict::SignIn => Err(Closed::SignIn),
            Verdict::Denied => Err(Closed::Denied),
        }
    }
}

/// Whether `rest` (a path below the link, built into `dir`) is served
/// to a caller [`Shares::open`] let in: the second check of every request
/// to a link, a page's or a guest's. An audience's link is checked
/// against the vault as it is now ([`Guard`]), then a file against the
/// link's layer ([`Shares::above`]).
async fn admit(
    shares: &Shares,
    share: &Share,
    audience: Option<&ShareAudience>,
    dir: &Path,
    rest: &str,
    listen: Listen,
) -> Admit {
    // A rebuild swaps the folder in with two renames; in between it is
    // not there for a moment. Wait that out first, so every check below
    // sees the folder that will answer.
    for _ in 0..SWAP_WAITS {
        if dir.is_dir() {
            break;
        }
        tokio::time::sleep(SWAP_WAIT).await;
    }
    if let Some(audience) = audience {
        let asked = {
            let (shares, share, audience, rest) = (
                shares.clone(),
                share.clone(),
                audience.clone(),
                rest.to_owned(),
            );
            blocking(move || shares.guard(&share, &audience, &rest)).await
        };
        match asked.unwrap_or(Guard::Refused) {
            Guard::Open => {}
            // Answered as a path that is nowhere is, so the link says
            // nothing of another audience's note or file.
            Guard::Refused => return Admit::Hidden,
            Guard::Building => return Admit::Building,
            Guard::Empty => return Admit::Empty,
        }
    }
    match needs_layer(shares, share, audience, dir, rest, listen).await {
        Hold::Serve => Admit::Open,
        Hold::Held(needs) => Admit::Held(needs),
        Hold::Hidden => Admit::Hidden,
        Hold::Building => Admit::Building,
    }
}

/// Whether the file at `rest` is held back from a link that plays to
/// `cap` ([`Shares::above`]).
async fn needs_layer(
    shares: &Shares,
    share: &Share,
    audience: Option<&ShareAudience>,
    dir: &Path,
    rest: &str,
    cap: Listen,
) -> Hold {
    if cap == Listen::Full {
        return Hold::Serve;
    }
    let (shares, share, audience, dir, rest) = (
        shares.clone(),
        share.clone(),
        audience.cloned(),
        dir.to_owned(),
        rest.to_owned(),
    );
    // Not answered: held back, never served.
    blocking(move || shares.above(&share, audience.as_ref(), &dir, &rest, cap))
        .await
        .unwrap_or(Hold::Held(Listen::Full))
}

/// `503`: the link is being built (or a file of it cannot be read now).
fn being_built() -> Response {
    let mut response = plain(
        StatusCode::SERVICE_UNAVAILABLE,
        "This link is being built. Try again in a moment.",
    );
    // When to ask again ([`private`] keeps it out of every cache).
    response
        .headers_mut()
        .insert(header::RETRY_AFTER, HeaderValue::from_static("5"));
    response
}

/// `403` for a file of layer `needs` on a link that plays to `cap`: the
/// layer named in the body and in `x-architect-layer`.
fn layer_refusal(cap: Listen, needs: Listen) -> Response {
    let mut response = (
        StatusCode::FORBIDDEN,
        format!(
            "This file is in the {needs} layer. This link plays to {cap}.",
            needs = needs.label(),
            cap = cap.label()
        ),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    headers.insert("x-architect-layer", HeaderValue::from_static(needs.label()));
    response
}

/// What every answer of a share carries: no referrer (the code stays off
/// other sites), not listed unless public, not cached by anybody else
/// when it is for signed-in people.
fn private(share: &Share, mut response: Response) -> Response {
    // A link that is being built is stored nowhere: what it answers next
    // is not what it answers now.
    let unavailable = response.status() == StatusCode::SERVICE_UNAVAILABLE;
    let headers = response.headers_mut();
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    if !matches!(share.audience, architect_spaces::Audience::Public) {
        headers.insert(
            "x-robots-tag",
            HeaderValue::from_static("noindex, nofollow"),
        );
    }
    let cache = if share.audience.signed_in() {
        "private, no-store"
    } else if unavailable {
        "no-store"
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

/// How long, and how many times, a request waits for a link's folder
/// being swapped in by a rebuild (in [`serve`], before any check).
const SWAP_WAIT: Duration = Duration::from_millis(20);
const SWAP_WAITS: usize = 10;

/// The built file at `rest` under `dir`. A file is streamed and
/// answers `Range` (`206`, `Accept-Ranges`), so the app's `<audio>` seeks
/// without downloading it (#511). On a share's own domain (`site`, a Site)
/// a folder answers its page and a path that is nowhere the site's 404
/// page; on a link, a plain `404`.
async fn file(dir: &Path, rest: &str, headers: &HeaderMap, site: bool) -> Response {
    let decoded = decode(rest);
    let safe = !decoded
        .split('/')
        .any(|s| s == ".." || s == "." || s.contains('\\') || s.contains('\0'));
    if !safe {
        return missing(dir, site).await;
    }
    let mut path = dir.join(decoded.trim_start_matches('/'));
    // On a Site, a folder answers with the page the builder wrote there
    // (it copies no vault file as `index.html`), marked so the site's
    // player can tell it from an attachment (`architect::host::BUILT_PAGE`).
    // On a link no page is served ([`is_page_file`]).
    let built = site && (decoded.ends_with('/') || path.is_dir());
    if built {
        path = path.join("index.html");
    }
    // Only a file of the link's built output, resolved: a symlink in it
    // is never followed out of it.
    match (dir.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(resolved)) if resolved.starts_with(&root) => path = resolved,
        (Err(_), _) => return being_built(),
        _ => return missing(dir, site).await,
    }
    if !path.is_file() {
        return missing(dir, site).await;
    }
    let kind = content_type(&path);
    if !kind.starts_with("text/html") {
        if let Ok(response) = architect::http::ranged::serve_file(headers, &path, kind).await {
            return response;
        }
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(&path)),
            );
            if built {
                response.headers_mut().insert(
                    header::HeaderName::from_static(architect::host::BUILT_PAGE.0),
                    HeaderValue::from_static(architect::host::BUILT_PAGE.1),
                );
            }
            response
        }
        Err(_) if !dir.exists() => being_built(),
        Err(_) => missing(dir, site).await,
    }
}

/// Nothing at a path: on a Site its 404 page, on a link [`gone`].
async fn missing(dir: &Path, site: bool) -> Response {
    if site { not_found(dir).await } else { gone() }
}

/// The answer for a path that is not in the link built into `dir`: its
/// site's 404 page, else a plain one. The same for anything not there,
/// whatever it is elsewhere.
async fn not_found(dir: &Path) -> Response {
    tokio::fs::read(dir.join("404.html")).await.map_or_else(
        |_| page(StatusCode::NOT_FOUND, "Not found", "<p>No such page.</p>"),
        |bytes| {
            let mut response = Response::new(Body::from(bytes));
            *response.status_mut() = StatusCode::NOT_FOUND;
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/html; charset=utf-8"),
            );
            response
        },
    )
}

fn content_type(path: &Path) -> &'static str {
    if let Some(media) = architect_sites::media::media_type(&path.to_string_lossy()) {
        return media;
    }
    match path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .as_deref()
    {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("txt" | "md" | "lrc") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// `_live`: an event each time the share is rebuilt or taken down, so an
/// open page reloads.
fn live(shares: &Shares, code: &str) -> Response {
    let Some(built) = shares.built(code) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let stream = futures::stream::unfold(built, |mut built| async move {
        built.changed().await.ok()?;
        Some((
            Ok::<_, std::convert::Infallible>(Event::default().data("rebuilt")),
            built,
        ))
    });
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// ── Files: the views and the archive (#510) ─────────────────────────

/// One link being answered: a share's own, or one audience's.
struct Link<'a> {
    share: &'a Share,
    audience: Option<&'a ShareAudience>,
    /// Where it is served: `/s/<code>/`, or `/` on a domain.
    mount: &'a str,
    /// How far it plays: the files above it are left out
    /// ([`Share::listen_for`]).
    listen: Listen,
}

impl Link<'_> {
    /// The link's code: the audience's, else the share's.
    fn code(&self) -> &str {
        self.audience.map_or(&self.share.code, |a| &a.code)
    }

    /// What the files page and the archive are called.
    fn title(&self) -> String {
        match (self.audience, self.share.title.as_deref()) {
            (Some(audience), Some(title)) => format!("{title}: {}", audience.name),
            (Some(audience), None) => audience.name.clone(),
            (None, Some(title)) => title.to_owned(),
            (None, None) => "Shared files".to_owned(),
        }
    }
}

/// What a link lists now, grouped.
struct Listed {
    entries: Arc<Vec<Entry>>,
    groups: Vec<Group>,
    /// Each listed note's page, as a path below the link.
    pages: HashMap<String, String>,
    /// The vault's folder: where the files are read from.
    root: PathBuf,
}

impl Shares {
    /// What `link` lists now, grouped `by`, read from the vault as it
    /// is, with no file above `cap`. Blocking ([`Self::view`]).
    fn listed(
        &self,
        share: &Share,
        audience: Option<&ShareAudience>,
        by: &Grouping,
        cap: Listen,
    ) -> Listed {
        let entries = self.listing(share, audience);
        let entries = if cap == Listen::Full {
            entries
        } else if let Some(placed) = self.placed(share) {
            Arc::new(within(&entries, &placed, cap))
        } else {
            // Where the files are is not known: none of them.
            Arc::new(
                entries
                    .iter()
                    .map(|e| Entry {
                        files: Vec::new(),
                        ..e.clone()
                    })
                    .collect(),
            )
        };
        let manifest = {
            let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
            table.get(&share.code).and_then(|held| {
                held.manifests
                    .get(audience.map_or(&share.code, |a| &a.code))
                    .cloned()
            })
        };
        let pages = entries
            .iter()
            .filter_map(|e| {
                let page = manifest.as_ref()?.page_of(&e.path)?;
                Some((e.path.clone(), page.to_owned()))
            })
            .collect();
        Listed {
            groups: listing::groups(&entries, by),
            entries,
            pages,
            root: self.0.backend.root(&share.vault).unwrap_or_default(),
        }
    }

    async fn listed_for(&self, link: &Link<'_>, by: &Grouping) -> Option<Listed> {
        let (shares, share, audience, by, cap) = (
            self.clone(),
            link.share.clone(),
            link.audience.cloned(),
            by.clone(),
            link.listen,
        );
        blocking(move || shares.listed(&share, audience.as_ref(), &by, cap)).await
    }
}

#[derive(serde::Deserialize)]
struct ViewQuery {
    #[serde(default)]
    by: String,
    group: Option<String>,
}

/// The view a files page or an archive was asked in: `?by=` (`order`
/// when unset) and `?group=`; `None` for a `by` that is no grouping.
fn view_of(query: &str) -> Option<(Grouping, Option<String>)> {
    let asked = serde_urlencoded_form::<ViewQuery>(query.as_bytes()).ok()?;
    Some((Grouping::parse(&asked.by)?, asked.group))
}

#[derive(serde::Deserialize)]
struct ListenQuery {
    listen: Option<String>,
}

/// `<link>_files.json` (#528): what the link serves, for a client that
/// opens a song over it. `listen` is the layer listed (`?listen=`, never
/// past the link's), `max` the most the link plays; each note with its
/// type, page and files; each file once, in list order, with its size,
/// sha256 (None when the vault has not hashed it yet), layer and URL.
async fn files_json(shares: &Shares, link: &Link<'_>, query: &str) -> Response {
    let asked = match serde_urlencoded_form::<ListenQuery>(query.as_bytes())
        .ok()
        .and_then(|q| q.listen)
        .map(|l| Listen::parse(&l))
    {
        None => link.listen,
        Some(Ok(l)) => l.min(link.listen),
        Some(Err(why)) => {
            let mut response = (
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "error": why }).to_string(),
            )
                .into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            return response;
        }
    };
    let (notes, files) = match listing(shares, link, asked).await {
        Ok(found) => found,
        Err(Admit::Building) => {
            let mut response = (
                StatusCode::SERVICE_UNAVAILABLE,
                serde_json::json!({ "error": "which layer each file is in is not known now (an extension's manifest or the vault does not read); try again later" }).to_string(),
            )
                .into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            return response;
        }
        Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    };
    let notes: Vec<serde_json::Value> = notes
        .into_iter()
        .map(|n| {
            serde_json::json!({
                "path": n.path,
                "title": n.title,
                "type": n.kind,
                "page": n.page,
                "files": n.files,
            })
        })
        .collect();
    let files: Vec<serde_json::Value> = files
        .into_iter()
        .map(|f| {
            serde_json::json!({
                "path": f.path,
                "url": f.url,
                "size": f.size,
                "sha256": f.sha256,
                "layer": f.layer,
                "note": f.note,
            })
        })
        .collect();
    let body = serde_json::json!({
        "link": link.mount,
        "title": link.title(),
        "listen": asked.label(),
        "max": link.listen.label(),
        "notes": notes,
        "files": files,
    });
    let mut response = (StatusCode::OK, body.to_string()).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    response
}

/// What `link` lists now at layer `asked` (never past the link's): its
/// notes in list order, each with its type, page and files, and each
/// file once, with its size, sha256, layer and URL. The listing of
/// `<link>_files.json` and of the link's guests ([`GuestLane`]) alike.
/// `Err(Admit::Building)` while where files are is not known (below
/// Full).
async fn listing(
    shares: &Shares,
    link: &Link<'_>,
    asked: Listen,
) -> Result<(Vec<ListedNote>, Vec<ListedFile>), Admit> {
    let at = Link {
        listen: asked.min(link.listen),
        ..*link
    };
    let Some(listed) = shares.listed_for(&at, &Grouping::Order).await else {
        return Err(Admit::Building);
    };
    let found = {
        let (shares, share) = (shares.clone(), link.share.clone());
        blocking(move || {
            let held: HashMap<String, (u64, String)> = shares
                .0
                .backend
                .manifest(&share.vault)
                .map(|m| {
                    m.files
                        .into_iter()
                        .map(|f| (f.path, (f.size, f.sha256)))
                        .collect()
                })
                .unwrap_or_default();
            (held, shares.placed(&share))
        })
        .await
    };
    let Some((held, placed)) = found else {
        return Err(Admit::Building);
    };
    if placed.is_none() && at.listen < Listen::Full {
        return Err(Admit::Building);
    }
    let base = link.mount.trim_end_matches('/');
    let mut notes = Vec::new();
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();
    // A collection's links resolve among the notes this link lists and
    // no others (#668): a guest never learns of a note outside it.
    let listed_paths: Vec<&str> = listed.entries.iter().map(|e| e.path.as_str()).collect();
    for entry in listed.entries.iter() {
        let item = architect_media::Item::read(&entry.path, &entry.properties);
        let sets = architect_media::Collection::read(&entry.properties)
            .map(|c| {
                c.resolve(&entry.path, listed_paths.iter().copied())
                    .sections
                    .into_iter()
                    .map(|(name, entries)| architect_spaces::ListedSet {
                        name,
                        notes: entries.into_iter().filter_map(|e| e.path).collect(),
                    })
                    .filter(|s| !s.notes.is_empty())
                    .collect()
            })
            .unwrap_or_default();
        notes.push(ListedNote {
            subtitle: item.subtitle.clone(),
            #[allow(
                clippy::as_conversions,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss
            )]
            duration_ms: item.duration.map(|d| (d * 1000.0).round() as u64),
            // Only media the link serves at this layer: one of its files.
            media: item
                .media_in(entry.files.iter().map(String::as_str))
                .map(str::to_owned),
            sets,
            path: entry.path.clone(),
            title: entry.title.clone(),
            kind: entry
                .properties
                .get("type")
                .and_then(|t| t.as_str())
                .map(str::to_owned),
            page: listed.pages.get(&entry.path).map(|p| format!("{base}{p}")),
            files: entry.files.clone(),
            // Only art the link serves at this layer: one of its files.
            art: entry
                .art()
                .filter(|a| entry.files.iter().any(|f| f == a))
                .map(str::to_owned),
        });
        for file in &entry.files {
            if !seen.insert(file.as_str()) {
                continue;
            }
            let (size, sha256) = held.get(file).map_or_else(
                || {
                    // Never a symlink's target's size.
                    let size = std::fs::symlink_metadata(listed.root.join(file))
                        .ok()
                        .filter(std::fs::Metadata::is_file)
                        .map_or(0, |m| m.len());
                    (size, None)
                },
                |(size, hash)| (*size, Some(hash.clone())),
            );
            files.push(ListedFile {
                path: file.clone(),
                url: format!("{base}/{}", architect_sites::encode_path(file)),
                size,
                sha256,
                layer: placed.as_ref().map(|p| {
                    p.get(file)
                        .copied()
                        .unwrap_or(Listen::Page)
                        .label()
                        .to_owned()
                }),
                note: entry.path.clone(),
            });
        }
    }
    Ok((notes, files))
}

// ── The link's guests (#629) ─────────────────────────────────────────

/// [`ShareGuest`], the read-only host service of a link's guests.
///
/// On the node at `POST /s/<code>/_guest/<method>`. Each call is made as
/// `Principal::Guest { link_id: <code> }` and checked by [`Shares::open`]
/// and [`admit`], the two checks every request for the link's pages and
/// files goes through: what a guest reads is what the page gives the
/// same visitor, as data.
#[derive(Clone)]
pub struct GuestLane(pub Shares);

/// The guest service's one wire.
impl architect::Services for GuestLane {
    fn layers() -> impl architect::Layer<Self> {
        architect::layers![architect_spaces::guest::Service]
    }
}

/// [`GuestFault`] for a path [`admit`] did not let through.
fn shut(admit: Admit) -> GuestFault {
    match admit {
        Admit::Held(needs) => GuestFault::Held(needs.label().to_owned()),
        Admit::Building => GuestFault::Building,
        Admit::Open | Admit::Hidden | Admit::Empty => GuestFault::Gone,
    }
}

impl GuestLane {
    /// The link `code` opens and its audience, open to the holder of
    /// `token` now.
    async fn opened(
        &self,
        code: &str,
        token: String,
    ) -> Result<(Share, Option<ShareAudience>), GuestFault> {
        let (share, audience) = Some(code)
            .filter(|c| is_code(c))
            .and_then(|c| self.0.link(c))
            .ok_or(GuestFault::Gone)?;
        // No token in the call: the browser's sign-in to this node, if it
        // sent one.
        let token = if token.is_empty() {
            GUEST_COOKIE
                .try_with(Clone::clone)
                .ok()
                .flatten()
                .unwrap_or_default()
        } else {
            token
        };
        match self.0.open(&share, Some(token)).await {
            Ok(_) => Ok((share, audience)),
            Err(Closed::Gone) => Err(GuestFault::Gone),
            Err(Closed::SignIn) => Err(GuestFault::SignIn),
            Err(Closed::Denied) => Err(GuestFault::Denied),
        }
    }

    /// Where the link's pages are built.
    fn dir(&self, share: &Share, audience: Option<&ShareAudience>) -> PathBuf {
        audience.map_or_else(
            || self.0.0.out.join(&share.code).join("site"),
            |a| self.0.audience_dir(&share.code, &a.code).join("site"),
        )
    }
}

/// Who a link is for, in a word.
const fn audience_word(audience: &architect_spaces::Audience) -> &'static str {
    match audience {
        architect_spaces::Audience::Public => "public",
        architect_spaces::Audience::Link => "link",
        architect_spaces::Audience::Org => "org",
        architect_spaces::Audience::Invited(_) => "invited",
    }
}

impl ShareGuest for GuestLane {
    async fn link(&self, code: String, token: String) -> Result<GuestLink, GuestFault> {
        let (share, audience) = self.opened(&code, token).await?;
        let mount = link_path(&code);
        let link = Link {
            share: &share,
            audience: audience.as_ref(),
            mount: &mount,
            listen: share.listen_for(audience.as_ref()),
        };
        let (notes, _) = listing(&self.0, &link, link.listen).await.map_err(shut)?;
        Ok(GuestLink {
            link: mount.clone(),
            title: link.title(),
            listen: link.listen.label().to_owned(),
            audience: audience_word(&share.audience).to_owned(),
            notes,
        })
    }

    async fn read(
        &self,
        code: String,
        token: String,
        path: String,
    ) -> Result<GuestNote, GuestFault> {
        let (share, audience) = self.opened(&code, token).await?;
        let mount = link_path(&code);
        let link = Link {
            share: &share,
            audience: audience.as_ref(),
            mount: &mount,
            listen: share.listen_for(audience.as_ref()),
        };
        // A note the link lists now, with a page of the link's.
        let (notes, files) = listing(&self.0, &link, link.listen).await.map_err(shut)?;
        // What a listing fence may name (#668): the notes the link lists
        // and the files it serves at this layer.
        let readable: std::collections::HashSet<String> = notes
            .iter()
            .map(|n| n.path.clone())
            .chain(files.iter().map(|f| f.path.clone()))
            .collect();
        let listed = notes
            .into_iter()
            .find(|n| n.path == path)
            .ok_or(GuestFault::Gone)?;
        let page = listed.page.clone().ok_or(GuestFault::Gone)?;
        let below = page
            .strip_prefix(mount.trim_end_matches('/'))
            .filter(|b| b.starts_with('/') && b.ends_with('/'))
            .ok_or(GuestFault::Gone)?
            .to_owned();
        // Admitted as its page is.
        let rest = format!("{below}index.html");
        let dir = self.dir(&share, audience.as_ref());
        match admit(&self.0, &share, audience.as_ref(), &dir, &rest, link.listen).await {
            Admit::Open => {}
            other => return Err(shut(other)),
        }
        // The page's data, from the build's data folder beside the served
        // one (no URL of the link reaches it), never through a symlink
        // (#674): one planted there, or swapped in since the build, is
        // refused.
        let dir = architect_sites::data_dir(&dir);
        let rel = format!(
            "{}{}",
            below.trim_start_matches('/'),
            architect_sites::plain::NOTE_FILE
        );
        let text = blocking(move || {
            let mut file = architect_sites::listing::open_own(&dir, &rel)?;
            let mut text = String::new();
            std::io::Read::read_to_string(&mut file, &mut text).map(|_| text)
        })
        .await
        .and_then(Result::ok)
        .ok_or(GuestFault::Building)?;
        let mut note: GuestNote = facet_json::from_str(&text).map_err(|_| GuestFault::Building)?;
        within_listing(&mut note, &|p| readable.contains(p));
        // What the view that draws its type may read of it (#648, #669),
        // decided here, by this node's islands, never by the caller: its
        // text (as the build wrote it for this link) only to a view that
        // reads the note, and the link's files only to one that reads
        // files too.
        let reads = architect_sites::island::note_reads(
            self.0.islands(),
            listed.kind.as_deref().unwrap_or_default(),
        );
        if !reads.text {
            note.source = None;
        }
        note.files = if reads.files {
            view_files(&listed, files)
        } else {
            Vec::new()
        };
        note.path = listed.path;
        note.kind = listed.kind;
        note.page = page;
        Ok(note)
    }

    async fn files(
        &self,
        code: String,
        token: String,
        listen: Option<String>,
    ) -> Result<Vec<ListedFile>, GuestFault> {
        let (share, audience) = self.opened(&code, token).await?;
        let mount = link_path(&code);
        let link = Link {
            share: &share,
            audience: audience.as_ref(),
            mount: &mount,
            listen: share.listen_for(audience.as_ref()),
        };
        let asked = match listen.as_deref().map(Listen::parse) {
            None => link.listen,
            Some(Ok(l)) => l.min(link.listen),
            // A layer that does not read asks for nothing more.
            Some(Err(_)) => Listen::Page,
        };
        let (_, files) = listing(&self.0, &link, asked).await.map_err(shut)?;
        Ok(files)
    }
}

/// `note` with every listing fence (`architect_media::is_listing_fence`,
/// a collection's rows as an extension writes them, #668) cut to what
/// the guest may read (`readable`: the link's notes and the files it
/// serves), in its words and in its text alike
/// (`architect_media::fence::within`): a row naming a note outside the
/// link is gone, title and all; a file the link holds back is emptied
/// from a row kept. A block an extension drew from such a fence at the
/// build is gone whole: its words are the extension's, not cut by row.
fn within_listing(note: &mut GuestNote, readable: &dyn Fn(&str) -> bool) {
    let listing = |b: &architect_spaces::PlainBlock| {
        b.info.as_deref().is_some_and(|i| {
            architect_media::is_listing_fence(i.strip_prefix("fence:").unwrap_or(i))
        })
    };
    note.blocks.retain_mut(|b| match b.kind {
        architect_spaces::BlockKind::Code if listing(b) => {
            b.text = architect_media::fence::rows_within(&b.text, readable, readable);
            !b.text.trim().is_empty()
        }
        architect_spaces::BlockKind::Drawn => !listing(b),
        _ => true,
    });
    if let Some(source) = note.source.as_mut() {
        *source = architect_media::fence::within(source, readable, readable);
    }
}

/// The files of the link a view on `note` may read
/// ([`GuestNote::files`]): the note's own first, then the rest of the
/// link's in list order (a setlist's view draws its songs' art), only
/// images and synced lyrics, within the caps of
/// [`architect_spaces::view::within_caps`]. `files` is the link's listing
/// at its layer: nothing past what the link serves its guest. A file the
/// vault has not hashed yet is left out: the app checks every file it is
/// handed against its `sha256`, and takes none without one.
fn view_files(note: &ListedNote, files: Vec<ListedFile>) -> Vec<ListedFile> {
    let (own, rest): (Vec<ListedFile>, Vec<ListedFile>) = files
        .into_iter()
        .filter(|f| f.sha256.is_some())
        .partition(|f| note.files.contains(&f.path));
    architect_spaces::view::within_caps(
        own.into_iter().chain(rest),
        |f| f.path.as_str(),
        |f| f.size,
    )
}

/// How much of a file is read, and sent, at a time.
const ARCHIVE_CHUNK: usize = 64 * 1024;

/// `<link>_archive.zip`: the files the link lists, or of one group of
/// the view asked for, as a zip written to the response while the files
/// are read.
async fn archive(shares: &Shares, link: &Link<'_>, query: &str) -> Response {
    let Some((by, group)) = view_of(query) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(listed) = shares.listed_for(link, &by).await else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let files: Vec<(String, PathBuf, String)> = listed
        .groups
        .iter()
        .filter(|g| group.as_ref().is_none_or(|want| *want == g.label))
        .flat_map(|g| g.files.iter())
        .map(|f| (f.archive.clone(), listed.root.join(&f.file), f.file.clone()))
        .collect();
    if files.is_empty() {
        return page(
            StatusCode::NOT_FOUND,
            "No files",
            "<p>There are no files to download here.</p>",
        );
    }
    let name = group.as_deref().filter(|g| !g.is_empty()).map_or_else(
        || link.title(),
        |group| format!("{} - {group}", link.title()),
    );
    let code = link.code().to_owned();
    let (tx, rx) = tokio::sync::mpsc::channel::<std::io::Result<Vec<u8>>>(4);
    let still = Still {
        shares: shares.clone(),
        code: code.clone(),
    };
    let root = listed.root;
    tokio::spawn(async move {
        if let Err(error) = write_archive(&root, files, &still, &tx).await {
            tracing::warn!(%code, %error, "a share archive stopped");
            // The download ends short of its directory: an archive that
            // does not open, never one that looks whole and is not.
            let _ = tx.send(Err(error)).await;
        }
    });
    let stream = futures::stream::unfold(rx, |mut rx| async move {
        rx.recv().await.map(|piece| (piece, rx))
    });
    let mut response = Response::new(Body::from_stream(stream));
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/zip"),
    );
    if let Ok(value) = HeaderValue::from_str(&attachment(&format!("{name}.zip"))) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// `path` as [`Shares::above`] compares spellings: folded
/// (`share_layers::fold`: one Unicode form, lower cased), `//` collapsed.
fn fold_path(path: &str) -> String {
    let joined: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    architect_sites::share_layers::fold(&joined.join("/"))
}

/// Whether a link still serves a file, asked by an archive in progress.
struct Still {
    shares: Shares,
    /// The link's code (a share's own, or an audience's).
    code: String,
}

impl Still {
    /// Whether the link is live and `file` (a vault path) not above its
    /// layer, now.
    async fn serves(&self, file: &str) -> bool {
        let Some((share, audience)) = self.shares.link(&self.code) else {
            return false;
        };
        if share.state(now()) != ShareState::Live {
            return false;
        }
        let cap = share.listen_for(audience.as_ref());
        if cap == Listen::Full {
            return true;
        }
        let (shares, file) = (self.shares.clone(), file.to_owned());
        blocking(move || {
            shares
                .placed(&share)
                .is_some_and(|placed| placed.get(&file).is_none_or(|layer| *layer <= cap))
        })
        .await
        .unwrap_or(false)
    }
}

/// Write `files` (each: its path in the archive, the file to read) to
/// `to` as a zip, a piece at a time. A file that is gone, or no longer
/// a plain file of the vault at `root` (swapped for a symlink since it
/// was listed), is left out: none is read through a symlink.
///
/// # Errors
///
/// A file could not be read through, or nobody is reading any more.
async fn write_archive(
    root: &Path,
    files: Vec<(String, PathBuf, String)>,
    still: &Still,
    to: &tokio::sync::mpsc::Sender<std::io::Result<Vec<u8>>>,
) -> std::io::Result<()> {
    use tokio::io::AsyncReadExt as _;
    let gone = |_| std::io::Error::new(std::io::ErrorKind::BrokenPipe, "the reader left");
    let mut zip = ZipWriter::new();
    for (name, path, vault_path) in files {
        // Decided when the archive began; asked again before each file,
        // so a revoke, an expiry or a lowered layer stops it there.
        if !still.serves(&vault_path).await {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the link no longer serves the rest",
            ));
        }
        // Opened only as the vault's own file, as it is now.
        let opened = {
            let (root, rel) = (root.to_path_buf(), vault_path.clone());
            blocking(move || architect_sites::listing::open_own(&root, &rel).ok())
                .await
                .flatten()
        };
        let Some(mut file) = opened.map(tokio::fs::File::from_std) else {
            tracing::debug!(path = %path.display(), "a shared file is gone or not the vault's; left out of the archive");
            continue;
        };
        let modified = file.metadata().await.ok().and_then(|m| m.modified().ok());
        to.send(Ok(zip.start(&name, modified)))
            .await
            .map_err(gone)?;
        loop {
            let mut chunk = vec![0_u8; ARCHIVE_CHUNK];
            let read = file.read(&mut chunk).await?;
            if read == 0 {
                break;
            }
            chunk.truncate(read);
            zip.data(&chunk);
            to.send(Ok(chunk)).await.map_err(gone)?;
        }
        to.send(Ok(zip.end())).await.map_err(gone)?;
    }
    to.send(Ok(zip.finish())).await.map_err(gone)
}

/// A `Content-Disposition` that saves as `name`: plain for old readers,
/// UTF-8 for the rest.
fn attachment(name: &str) -> String {
    let plain: String = name
        .chars()
        .map(|c| {
            if c.is_ascii() && !c.is_ascii_control() && c != '"' && c != '\\' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!(
        "attachment; filename=\"{plain}\"; filename*=UTF-8''{}",
        encode(name).replace('/', "%2F")
    )
}

// ── Sign-in ──────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
struct To {
    #[serde(default)]
    to: String,
}

/// `/s/_signin`, `/s/_signin/wait`, `/s/_signout` (`rest` is what
/// follows the `_`).
async fn signin(shares: &Shares, rest: &str, request: Request) -> Response {
    let query = request.uri().query().unwrap_or_default().to_owned();
    let to = serde_urlencoded_form::<To>(query.as_bytes())
        .map_or_else(|_| "/".to_owned(), |t| safe_to(&t.to));
    let secure = is_https(request.headers(), shares.0.issuer_origin.as_deref());
    match rest {
        // Sign-out is a form posted from the node's own page, its Origin
        // checked: on every browser, whether it sends fetch metadata or not
        // (Safari 15.4 to 16.3 sends none). A share page's frames cannot
        // post (no `allow-forms`, `connect-src 'none'`), and their Origin
        // is `null`.
        "signout" => signout(request.method(), request.headers(), &to, secure),
        // Sign-in (it starts a device sign-in; it signs no one out) is
        // answered only to the node's own pages or the visitor's typing.
        "signin" if !same_site(request.headers()) => page(
            StatusCode::FORBIDDEN,
            "Not from here",
            "<p>Sign in from the link's own page.</p>",
        ),
        "signin" => start_signin(shares, to, secure).await,
        "signin/wait" => wait_signin(shares, request.headers(), secure).await,
        _ => page(StatusCode::NOT_FOUND, "Not found", "<p>No such page.</p>"),
    }
}

/// Whether a request is the node's own (`Sec-Fetch-Site: same-origin`),
/// or the visitor's typing or bookmark (`none`); a browser that sends no
/// fetch metadata at all is taken at its word (it sends none for any).
fn same_site(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_none_or(|site| site == "same-origin" || site == "none")
}

/// A form that signs the visitor out and goes back to `to` (the only way
/// out: [`signout`] clears nothing on a GET).
fn signout_form(to: &str) -> String {
    format!(
        "<form method=\"post\" action=\"/s/_signout?to={}\"><button class=\"button\" type=\"submit\">Sign out</button></form>",
        escape(&urlencoding_path(to))
    )
}

/// `to` as a query value: what a path may hold, the rest percent-encoded.
fn urlencoding_path(to: &str) -> String {
    to.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// `/s/_signout`. A GET is a page asking, with the form ([`signout_form`]);
/// a POST from this node's own page (its `Origin` this node's: absent,
/// `null` or another site's is refused) clears the session cookie and goes
/// back to `to`. Nothing else.
fn signout(method: &Method, headers: &HeaderMap, to: &str, secure: bool) -> Response {
    if method == Method::GET || method == Method::HEAD {
        return page(
            StatusCode::OK,
            "Sign out",
            &format!(
                "<p>Sign out of this node's links on this browser?</p>{}<p><a href=\"{}\">Stay signed in</a></p>",
                signout_form(to),
                escape(to)
            ),
        );
    }
    if method != Method::POST {
        let mut response = page(
            StatusCode::METHOD_NOT_ALLOWED,
            "Not allowed",
            "<p>Only GET and POST.</p>",
        );
        response
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD, POST"));
        return response;
    }
    if !own_origin(headers, secure) {
        return page(
            StatusCode::FORBIDDEN,
            "Not from here",
            "<p>Sign out from the link's own page.</p>",
        );
    }
    let mut response = Redirect::to(to).into_response();
    set_cookie(&mut response, SESSION_COOKIE, "", "/", 0, secure);
    response
}

/// Whether a request's `Origin` is this node (its `Host`, over https when
/// `secure`): never when it has none, or `null` (a sandboxed frame).
fn own_origin(headers: &HeaderMap, secure: bool) -> bool {
    let header = |name| headers.get(name).and_then(|v| v.to_str().ok());
    let (Some(origin), Some(host)) = (header(header::ORIGIN), header(header::HOST)) else {
        return false;
    };
    let scheme = if secure { "https" } else { "http" };
    origin.eq_ignore_ascii_case(&format!("{scheme}://{host}"))
}

/// Why an issuer call failed when it got no answer within the lane's
/// timeout (`IssuerLane::within`).
const ISSUER_SILENT: &str = "the issuer did not answer in time";

async fn start_signin(shares: &Shares, to: String, secure: bool) -> Response {
    let (Some(issuer), Some(origin)) = (&shares.0.issuer, &shares.0.issuer_origin) else {
        return page(
            StatusCode::NOT_FOUND,
            "No sign-in",
            "<p>This node signs nobody in.</p>",
        );
    };
    let started = issuer
        .within(async {
            match issuer.client::<auth_proto::AuthServiceClient>().await {
                Ok(auth) => auth
                    .start_device_sign_in("architect-share".to_owned())
                    .await
                    .map_err(|e| e.to_string()),
                Err(e) => Err(e),
            }
        })
        .await
        .unwrap_or_else(|| Err(ISSUER_SILENT.to_owned()));
    let started = match started {
        Ok(s) => s,
        Err(why) => {
            tracing::warn!(%why, "a share sign-in did not start");
            return page(
                StatusCode::BAD_GATEWAY,
                "Sign-in unavailable",
                "<p>The sign-in server did not answer. Try again in a moment.</p>",
            );
        }
    };
    let nonce = code_from({
        let mut b = [0_u8; 10];
        b.copy_from_slice(
            uuid::Uuid::new_v4()
                .as_bytes()
                .get(..10)
                .unwrap_or(&[0; 10]),
        );
        b
    });
    let lasts = Duration::from_secs(
        u64::try_from(started.expires_in_seconds.clamp(60, 1800)).unwrap_or(600),
    );
    let until = Instant::now()
        .checked_add(lasts)
        .unwrap_or_else(Instant::now);
    {
        let mut pending = shares
            .0
            .signins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        pending.retain(|_, p| p.until > Instant::now());
        pending.insert(
            nonce.clone(),
            Pending {
                device_code: started.device_code,
                to,
                until,
            },
        );
    }
    let approve = if started.verification_uri_complete.starts_with('/') {
        format!("{origin}{}", started.verification_uri_complete)
    } else {
        started.verification_uri_complete
    };
    let mut response = waiting_page(&approve, &started.user_code, started.interval_seconds);
    set_cookie(
        &mut response,
        SIGNIN_COOKIE,
        &nonce,
        SIGNIN_PATH,
        1800,
        secure,
    );
    response
}

fn waiting_page(approve: &str, user_code: &str, interval: i64) -> Response {
    let body = format!(
        "<p>Approve this sign-in at your account, signed in there:</p><p><a class=\"button\" href=\"{0}\" target=\"_blank\" rel=\"noopener\">Approve sign-in</a></p><p>The code is <b>{1}</b>. This page goes on by itself once you approve.</p>",
        escape(approve),
        escape(user_code)
    );
    let mut response = page(StatusCode::OK, "Sign in", &body);
    let refresh = format!("{}; url={SIGNIN_PATH}/wait", interval.clamp(2, 10));
    if let Ok(value) = HeaderValue::from_str(&refresh) {
        response.headers_mut().insert("refresh", value);
    }
    response
}

async fn wait_signin(shares: &Shares, headers: &HeaderMap, secure: bool) -> Response {
    let Some(nonce) = cookie(headers, SIGNIN_COOKIE) else {
        return Redirect::to("/").into_response();
    };
    let pending = {
        let pending = shares
            .0
            .signins
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        pending
            .get(&nonce)
            .filter(|p| p.until > Instant::now())
            .map(|p| (p.device_code.clone(), p.to.clone()))
    };
    let (Some((device_code, to)), Some(issuer)) = (pending, &shares.0.issuer) else {
        return page(
            StatusCode::GONE,
            "Sign-in expired",
            "<p>That sign-in ran out. Open the link again to start over.</p>",
        );
    };
    let polled = issuer
        .within(async {
            match issuer.client::<auth_proto::AuthServiceClient>().await {
                Ok(auth) => auth
                    .poll_device_sign_in(device_code, Some("architect-share".to_owned()))
                    .await
                    .map_err(|e| format!("{e:?}")),
                Err(e) => Err(e),
            }
        })
        .await
        .unwrap_or_else(|| Err(ISSUER_SILENT.to_owned()));
    match polled {
        Ok(bundle) => {
            shares
                .0
                .signins
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&nonce);
            let mut response = Redirect::to(&to).into_response();
            let age = bundle
                .session
                .expires_at
                .timestamp()
                .saturating_sub(now())
                .clamp(60, 30 * 86_400);
            set_cookie(
                &mut response,
                SESSION_COOKIE,
                &bundle.token,
                "/",
                age,
                secure,
            );
            set_cookie(&mut response, SIGNIN_COOKIE, "", SIGNIN_PATH, 0, secure);
            response
        }
        Err(why) if why.contains("VerificationRequired") || why.contains("slow_down") => {
            let mut response = page(
                StatusCode::OK,
                "Sign in",
                "<p>Waiting for you to approve the sign-in…</p>",
            );
            response.headers_mut().insert(
                "refresh",
                HeaderValue::from_static("3; url=/s/_signin/wait"),
            );
            response
        }
        // No answer is not a refusal: the sign-in stays pending and the
        // page asks again.
        Err(why) if why == ISSUER_SILENT => {
            tracing::warn!("a share sign-in's poll got no answer from the issuer");
            let mut response = page(
                StatusCode::BAD_GATEWAY,
                "Sign-in unavailable",
                "<p>The sign-in server did not answer. This page tries again by itself.</p>",
            );
            response.headers_mut().insert(
                "refresh",
                HeaderValue::from_static("5; url=/s/_signin/wait"),
            );
            response
        }
        Err(why) => {
            tracing::debug!(%why, "a share sign-in was refused");
            page(
                StatusCode::FORBIDDEN,
                "Not signed in",
                "<p>The sign-in was refused or ran out. Open the link again to start over.</p>",
            )
        }
    }
}

// ── Small pieces ─────────────────────────────────────────────────────

fn set_cookie(
    response: &mut Response,
    name: &str,
    value: &str,
    path: &str,
    age: i64,
    secure: bool,
) {
    let secure = if secure { "; Secure" } else { "" };
    let line =
        format!("{name}={value}; Path={path}; Max-Age={age}; HttpOnly; SameSite=Lax{secure}");
    if let Ok(v) = HeaderValue::from_str(&line) {
        response.headers_mut().append(header::SET_COOKIE, v);
    }
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn is_https(headers: &HeaderMap, issuer: Option<&str>) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map_or_else(
            || issuer.is_some_and(|i| i.starts_with("https://")),
            |p| p.eq_ignore_ascii_case("https"),
        )
}

/// Only a path on this node: never another origin. A path from `/`, its
/// second character not another `/` or a backslash, and no control character
/// (a browser drops a tab or a newline, so `/\t/evil` is `//evil`) or
/// backslash (read as `/`) anywhere; anything else is `/`. What is kept
/// goes out percent-encoded past ASCII, so it is always a header's value.
fn safe_to(to: &str) -> String {
    let mut chars = to.chars();
    let ok = chars.next() == Some('/')
        && !matches!(chars.next(), Some('/' | '\\'))
        && !to.chars().any(|c| c.is_control() || c == '\\');
    if !ok {
        return "/".to_owned();
    }
    to.bytes()
        .map(|b| {
            if b.is_ascii_graphic() {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn serde_urlencoded_form<T: serde::de::DeserializeOwned>(bytes: &[u8]) -> Result<T, String> {
    architect::http::axum::extract::Query::<T>::try_from_uri(
        &format!("/?{}", String::from_utf8_lossy(bytes))
            .parse()
            .map_err(|e: architect::http::axum::http::uri::InvalidUri| e.to_string())?,
    )
    .map(|q| q.0)
    .map_err(|e| e.to_string())
}

fn decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let mut rest = s.as_bytes();
    while let Some((&b, tail)) = rest.split_first() {
        let hex = tail
            .get(..2)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) if b == b'%' => {
                out.push(byte);
                rest = tail.get(2..).unwrap_or_default();
            }
            _ => {
                out.push(b);
                rest = tail;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// A small page of the node's own (sign-in, not found), never a share's.
fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>{t}</title><style>body{{font:16px/1.5 system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem;color:#222}}@media(prefers-color-scheme:dark){{body{{background:#111;color:#eee}}}}.button{{display:inline-block;padding:.5rem 1rem;border-radius:.5rem;background:#3b6ef5;color:#fff;text-decoration:none}}</style></head><body><h1>{t}</h1>{body}</body></html>",
        t = escape(title)
    );
    let mut response = (status, html).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    // `same-origin`, not `no-referrer`: a form posted from a `no-referrer`
    // page carries `Origin: null` even to its own origin, and sign-out's
    // own form (`signout_form`) must say it is this node's. The referrer
    // still never leaves the node.
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("same-origin"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An issuer that takes connections and never answers.
    async fn stalled_issuer() -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let held = tokio::spawn(async move {
            // Each connection held open, unanswered, until the test ends.
            while let Ok((socket, _)) = listener.accept().await {
                tokio::spawn(async move {
                    let _held = socket;
                    std::future::pending::<()>().await;
                });
            }
        });
        (url, held)
    }

    /// A stalled issuer fails share sign-ins closed, promptly: a signed-in
    /// viewer is anonymous, and starting or polling a sign-in answers that
    /// the sign-in server did not answer, each within the lane's timeout
    /// rather than never.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stalled_issuer_fails_share_sign_ins_closed_within_the_timeout() {
        let (issuer, held) = stalled_issuer().await;
        let tmp = tempfile::tempdir().unwrap();
        let backend = Backend::single("notes", tmp.path().join("vault")).unwrap();
        let shares = Shares::new(
            backend,
            tmp.path(),
            None,
            Some(&issuer),
            architect_sites::Blocks::default(),
            LayerRules::default(),
        );
        let bound = super::super::org::ISSUER_TIMEOUT + Duration::from_secs(5);

        let nonce = "stalled-test".to_owned();
        shares
            .0
            .signins
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                nonce.clone(),
                Pending {
                    device_code: "device".into(),
                    to: "/s/x/".into(),
                    until: Instant::now() + Duration::from_secs(60),
                },
            );
        let mut signed_in = HeaderMap::new();
        signed_in.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer not-a-real-token"),
        );
        let mut waiting = HeaderMap::new();
        waiting.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{SIGNIN_COOKIE}={nonce}")).unwrap(),
        );
        let (viewer, started, polled) = tokio::join!(
            tokio::time::timeout(bound, shares.viewer_of(bearer(&signed_in))),
            tokio::time::timeout(bound, start_signin(&shares, "/s/x/".into(), false)),
            tokio::time::timeout(bound, wait_signin(&shares, &waiting, false)),
        );
        let viewer = viewer.expect("a viewer's sign-in hung on a stalled issuer");
        assert!(matches!(viewer, Viewer::Anonymous));
        let started = started.expect("starting a share sign-in hung on a stalled issuer");
        assert_eq!(started.status(), StatusCode::BAD_GATEWAY);
        let polled = polled.expect("polling a share sign-in hung on a stalled issuer");
        assert_eq!(polled.status(), StatusCode::BAD_GATEWAY);
        assert!(
            shares
                .0
                .signins
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key(&nonce),
            "no answer is not a refusal: the sign-in stays pending"
        );
        held.abort();
    }

    /// Signing in is not something a share page's frame, or another site,
    /// can make a visitor start by sending them there.
    #[test]
    fn sign_in_answers_only_the_nodes_own_requests() {
        let with = |site: Option<&str>| {
            let mut h = HeaderMap::new();
            if let Some(site) = site {
                h.insert("sec-fetch-site", HeaderValue::from_str(site).unwrap());
            }
            same_site(&h)
        };
        assert!(with(Some("same-origin")) && with(Some("none")) && with(None));
        assert!(!with(Some("cross-site")) && !with(Some("same-site")));
    }

    /// The node's own pages (sign-out's among them) send a referrer only to
    /// the node: their forms' posts then say their Origin (`no-referrer`
    /// would make it `null`, refused).
    #[test]
    fn the_nodes_own_pages_keep_their_origin_for_their_forms() {
        let r = page(StatusCode::OK, "Sign out", &signout_form("/"));
        assert_eq!(
            r.headers().get(header::REFERRER_POLICY).unwrap(),
            "same-origin"
        );
    }

    /// Signing out is a form posted from the node's own page: a GET (a
    /// link, a frame navigating, a browser that sends no fetch metadata)
    /// never clears the session; a POST does only with this node's Origin.
    #[test]
    fn sign_out_is_a_post_from_the_nodes_own_page() {
        let headers = |origin: Option<&str>| {
            let mut h = HeaderMap::new();
            h.insert(header::HOST, HeaderValue::from_static("node.test:8080"));
            if let Some(o) = origin {
                h.insert(header::ORIGIN, HeaderValue::from_str(o).unwrap());
            }
            h
        };
        let clears = |r: &Response| {
            r.headers()
                .get_all(header::SET_COOKIE)
                .iter()
                .any(|c| c.to_str().unwrap_or_default().starts_with(SESSION_COOKIE))
        };
        // GET (and HEAD): a page with the form, nothing cleared, whatever
        // the request says of itself.
        for method in [Method::GET, Method::HEAD] {
            for origin in [None, Some("http://node.test:8080")] {
                let r = signout(&method, &headers(origin), "/s/abc/", false);
                assert_eq!(r.status(), StatusCode::OK);
                assert!(!clears(&r), "{method} never signs out");
            }
        }
        // POST from anywhere but this node: refused.
        for origin in [
            None,
            Some("null"),
            Some("https://evil.example"),
            Some("https://node.test:8080"),
        ] {
            let r = signout(&Method::POST, &headers(origin), "/", false);
            assert_eq!(r.status(), StatusCode::FORBIDDEN, "{origin:?}");
            assert!(!clears(&r), "{origin:?}");
        }
        // The page's own form: signed out, back where it was.
        let r = signout(
            &Method::POST,
            &headers(Some("http://node.test:8080")),
            "/s/abc/",
            false,
        );
        assert!(clears(&r));
        assert_eq!(r.headers().get(header::LOCATION).unwrap(), "/s/abc/");
        assert_eq!(
            signout(
                &Method::PUT,
                &headers(Some("http://node.test:8080")),
                "/",
                false
            )
            .status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        // The form posts there, its way back kept.
        let form = signout_form("/s/a b/");
        assert!(
            form.contains("method=\"post\" action=\"/s/_signout?to=/s/a%20b/\""),
            "{form}"
        );
    }

    #[test]
    fn paths_and_cookies() {
        assert_eq!(safe_to("//evil.example"), "/");
        assert_eq!(safe_to("https://evil.example"), "/");
        assert_eq!(safe_to("/abc/x/"), "/abc/x/");
        assert_eq!(safe_to("/s/a b/é?x=1"), "/s/a%20b/%C3%A9?x=1");
        // Every way to another origin a browser would follow.
        for evil in [
            "/\t/evil.example",
            "/\n/evil.example",
            "/\r/evil.example",
            "/x\u{0}/y",
            "\\/evil.example",
            "//evil.example",
            "/\\evil.example",
            "https:evil.example",
            "javascript:alert(1)",
            " /evil",
            "",
        ] {
            assert_eq!(safe_to(evil), "/", "{evil:?}");
        }
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; architect_session=tok"),
        );
        assert_eq!(cookie(&headers, SESSION_COOKIE).as_deref(), Some("tok"));
        assert_eq!(decode("/a%20b/"), "/a b/");
        assert_eq!(
            view_of("by=property%3Ainstrument&group=violin"),
            Some((
                Grouping::Property("instrument".to_owned()),
                Some("violin".to_owned())
            ))
        );
        assert_eq!(view_of(""), Some((Grouping::Order, None)));
        assert_eq!(view_of("by=shuffle"), None);
        assert_eq!(
            attachment("Carols: violín.zip"),
            "attachment; filename=\"Carols: viol_n.zip\"; filename*=UTF-8''Carols%3A%20viol%C3%ADn.zip"
        );
        assert_eq!(encode("/a b/"), "/a%20b/");
    }
}
