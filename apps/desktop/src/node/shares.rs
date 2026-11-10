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
//! Nothing outside a share's scope is ever built into it, so nothing
//! outside it can be served: the site is built from the scope's notes
//! alone, its links to other notes are plain text, and an attachment is
//! there only when a note in the scope links to it.
//!
//! Sign-in (`/s/_signin`) is the issuer's device sign-in: the node
//! starts one, the person approves it at the issuer, signed in there,
//! and the node sets its session as an `HttpOnly` cookie. The node never
//! sees a password.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use architect::http::axum::body::Body;
use architect::http::axum::extract::{Request, State};
use architect::http::axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use architect::http::axum::middleware::Next;
use architect::http::axum::response::sse::{Event, KeepAlive, Sse};
use architect::http::axum::response::{IntoResponse, Redirect, Response};
use architect_orgs::server::{Directory as _, IdentityServer};
use architect_permissions::Principal;
use architect_spaces::share::{
    LINKS, SHARES_FOLDER, code_from, comments_note_path, is_code, link_path, share_note_path,
    validate, validate_domain,
};
use architect_spaces::{
    NewShare, Share, ShareChange, ShareComment, ShareFault, ShareLinks, ShareScope, ShareState,
    Verdict, Viewer, comments_note, parse_comments, parse_share_note, share_note,
};
use tokio::sync::{Notify, watch};
use vault_live::Backend;
use vault_proto::{IfMatch, VaultSync as _};

use super::org::{IssuerLane, NodeOrg};

/// The cookie a signed-in viewer's issuer session is kept in.
pub const SESSION_COOKIE: &str = "architect_session";
/// The comments section's start: its look, then its heading. The site's
/// own tokens (`--site-*`) colour it, so it follows the theme.
const COMMENTS_HEAD: &str = "<style>.share-comments{max-width:42rem;margin:2rem auto;padding:0 1rem}.share-comment{border-top:1px solid var(--site-rule,#8884);padding:.5rem 0}.share-comments form{display:flex;flex-direction:column;gap:.5rem;margin-top:1rem}.share-comments input,.share-comments textarea{font:inherit;padding:.5rem;border-radius:.4rem;border:1px solid var(--site-rule,#8886);background:transparent;color:inherit}.share-comments textarea{min-height:5rem}.share-comments button{align-self:flex-start;font:inherit;padding:.4rem 1rem;border-radius:.4rem;border:0;background:var(--site-accent,#3b6ef5);color:#fff}</style><section id=\"comments\" class=\"share-comments\"><h2>Comments</h2>";
/// The cookie a sign-in in progress is kept in, and its path.
const SIGNIN_COOKIE: &str = "architect_signin";
const SIGNIN_PATH: &str = "/s/_signin";
/// How long the vault is left to settle before the shares are rebuilt.
const SETTLE: Duration = Duration::from_millis(400);
/// How long a resolved viewer is trusted before the issuer is asked again.
const VIEWER_TTL: Duration = Duration::from_secs(60);
/// The most comments a share keeps; the longest one.
const MAX_COMMENTS: usize = 2000;
const MAX_COMMENT: usize = 4000;

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
    /// (#488).
    #[must_use]
    pub fn new(
        backend: Backend,
        data: &Path,
        org: Option<NodeOrg>,
        issuer: Option<&str>,
        blocks: architect_sites::Blocks,
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
        }))
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
                    Some(share) if share.code == name && share.vault == vault => {
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
                    table.insert(
                        code,
                        Held {
                            share,
                            built: watch::channel(0).0,
                        },
                    );
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
        let mut mounts = vec![(share.path(), at.join("site"))];
        if share.domain.is_some() {
            mounts.push(("/".to_owned(), at.join("domain")));
        } else {
            let _ = std::fs::remove_dir_all(at.join("domain"));
        }
        for (base, out) in mounts {
            let mut opts =
                architect_sites::Options::new(&root, share.title.clone().unwrap_or_default());
            opts.live_reload = Some(format!("{base}_live"));
            opts.base = base;
            opts.scope = scope(&share.scope);
            match architect_sites::build(&opts, &self.0.blocks, &out) {
                Ok(report) => {
                    tracing::debug!(code = %share.code, pages = report.pages, "a share built");
                }
                Err(error) => {
                    tracing::warn!(code = %share.code, %error, "a share did not build");
                    let _ = std::fs::remove_dir_all(&out);
                }
            }
        }
        let table = self.0.table.read().unwrap_or_else(PoisonError::into_inner);
        if let Some(held) = table.get(&share.code) {
            held.built.send_modify(|v| *v = v.wrapping_add(1));
        }
    }

    /// Keep `vault`'s shares read and built: now, then whenever the vault
    /// changes or a share of it is made or changed here.
    pub async fn keep_built(self, vault: String) {
        let mut changes = self.0.backend.channel(&vault).await.subscribe();
        let kick = self.kick(&vault);
        let pass = {
            let (me, vault) = (self.clone(), vault.clone());
            move || {
                me.load(&vault);
                me.build_all(&vault);
            }
        };
        let _ = tokio::task::spawn_blocking(pass.clone()).await;
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
            }
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
        {
            let mut table = self.0.table.write().unwrap_or_else(PoisonError::into_inner);
            match table.get_mut(&share.code) {
                Some(held) => held.share = share.clone(),
                None => {
                    table.insert(
                        share.code.clone(),
                        Held {
                            share: share.clone(),
                            built: watch::channel(0).0,
                        },
                    );
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
        let code = loop {
            let random = uuid::Uuid::new_v4();
            let mut bytes = [0_u8; 10];
            bytes.copy_from_slice(random.as_bytes().get(..10).unwrap_or(&[0; 10]));
            let code = code_from(bytes);
            if self.get(&code).is_none() {
                break code;
            }
        };
        self.save(Share {
            code,
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

    async fn add_comment(&self, share: &Share, comment: ShareComment) -> Result<(), ShareFault> {
        let mut all = self.comments(&share.code);
        all.push(comment);
        if all.len() > MAX_COMMENTS {
            return Err(ShareFault::Invalid(
                "this share has all the comments it can take".to_owned(),
            ));
        }
        let text = comments_note(&share.code, &all);
        put(
            self.0.backend.clone(),
            share.vault.clone(),
            comments_note_path(&share.code),
            text,
        )
        .await
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

    /// Who is asking: the issuer session in the bearer or the cookie,
    /// resolved by the issuer (cached a minute).
    async fn viewer(&self, headers: &HeaderMap) -> Viewer {
        let Some(token) = bearer(headers).or_else(|| cookie(headers, SESSION_COOKIE)) else {
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
        let user = match issuer.client::<auth_proto::AuthServiceClient>().await {
            Ok(auth) => auth.whoami(token.clone()).await.map_err(|e| e.to_string()),
            Err(e) => Err(e),
        };
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
        let member = match (
            &self.0.org,
            issuer
                .client::<auth_proto::OrganizationServiceClient>()
                .await,
        ) {
            (Some(org), Ok(client)) => IdentityServer(client)
                .orgs(&token)
                .await
                .is_ok_and(|orgs| orgs.iter().any(|o| o.id == org.id)),
            _ => false,
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
/// `/s/…` is `404` here, never the node's other routes).
pub async fn gate(State(shares): State<Shares>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    if let Some(code) = shares.at_host(&host) {
        let rest = request.uri().path().to_owned();
        return serve(&shares, &code, &rest, "/", request).await;
    }
    let path = request.uri().path().to_owned();
    let Some(below) = path
        .strip_prefix(LINKS)
        .filter(|r| r.is_empty() || r.starts_with('/'))
    else {
        return next.run(request).await;
    };
    let below = below.trim_start_matches('/');
    if let Some(rest) = below.strip_prefix('_') {
        return signin(&shares, rest, request).await;
    }
    let mut parts = below.splitn(2, '/');
    let first = parts.next().unwrap_or_default();
    if !is_code(first) || shares.get(first).is_none() {
        return page(
            StatusCode::NOT_FOUND,
            "Not found",
            "<p>There is nothing at this link.</p>",
        );
    }
    let mount = link_path(first);
    if !below.contains('/') {
        return Redirect::permanent(&mount).into_response();
    }
    let rest = format!("/{}", parts.next().unwrap_or_default());
    serve(&shares, first, &rest, &mount, request).await
}

/// Answer `rest` (a path below the link, from `/`) of share `code`,
/// whose link is at `mount`.
async fn serve(shares: &Shares, code: &str, rest: &str, mount: &str, request: Request) -> Response {
    let Some(share) = shares
        .get(code)
        .filter(|s| s.state(now()) == ShareState::Live)
    else {
        return page(
            StatusCode::NOT_FOUND,
            "Not found",
            "<p>There is nothing at this link.</p>",
        );
    };
    let viewer = if share.audience.signed_in() {
        shares.viewer(request.headers()).await
    } else {
        Viewer::Anonymous
    };
    match share.admits(&viewer) {
        Verdict::Open => {}
        Verdict::SignIn => {
            let to = format!("{}{}", mount.trim_end_matches('/'), rest);
            let body = format!(
                "<p>This link is for {}. Sign in to open it.</p><p><a class=\"button\" href=\"{LINKS}/_signin?to={}\">Sign in</a></p>",
                if matches!(share.audience, architect_spaces::Audience::Org) {
                    "the members of an organization"
                } else {
                    "the people it was shared with"
                },
                escape(&encode(&to))
            );
            return page(StatusCode::UNAUTHORIZED, "Sign in", &body);
        }
        Verdict::Denied => {
            return page(
                StatusCode::FORBIDDEN,
                "Not shared with you",
                "<p>This link was not shared with the account you are signed in as.</p><p><a href=\"/s/_signout?to=/\">Sign out</a></p>",
            );
        }
    }
    let base = if mount == "/" { "domain" } else { "site" };
    let dir = shares.0.out.join(code).join(base);
    let method = request.method().clone();
    match rest {
        "/_live" => live(shares, code),
        "/_comment" if method == architect::http::axum::http::Method::POST => {
            comment(shares, &share, &viewer, mount, request).await
        }
        _ => {
            let mut response = file(&dir, rest).await;
            if share.comments()
                && response
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .is_some_and(|t| t.as_bytes().starts_with(b"text/html"))
            {
                response = with_comments(shares, &share, rest, mount, response).await;
            }
            private(&share, response)
        }
    }
}

/// What every answer of a share carries: no referrer (the code stays off
/// other sites), not listed unless public, not cached by anybody else
/// when it is for signed-in people.
fn private(share: &Share, mut response: Response) -> Response {
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
    } else {
        "no-cache"
    };
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

/// The built file at `rest` under `dir`, or the site's 404 page.
async fn file(dir: &Path, rest: &str) -> Response {
    let decoded = decode(rest);
    let safe = !decoded
        .split('/')
        .any(|s| s == ".." || s == "." || s.contains('\\') || s.contains('\0'));
    if !safe {
        return page(StatusCode::NOT_FOUND, "Not found", "<p>No such page.</p>");
    }
    let mut path = dir.join(decoded.trim_start_matches('/'));
    if decoded.ends_with('/') || path.is_dir() {
        path = path.join("index.html");
    }
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut response = Response::new(Body::from(bytes));
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(&path)),
            );
            response
        }
        Err(_) if !dir.exists() => page(
            StatusCode::SERVICE_UNAVAILABLE,
            "Being built",
            "<p>This link's pages are being built. Try again in a moment.</p>",
        ),
        Err(_) => tokio::fs::read(dir.join("404.html")).await.map_or_else(
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
        ),
    }
}

fn content_type(path: &Path) -> &'static str {
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
        Some("txt" | "md") => "text/plain; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("pdf") => "application/pdf",
        Some("woff2") => "font/woff2",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        Some("mp4") => "video/mp4",
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

#[derive(serde::Deserialize)]
struct CommentForm {
    page: String,
    text: String,
    #[serde(default)]
    name: String,
}

/// `POST _comment`: a comment on a page, for a share that takes them.
async fn comment(
    shares: &Shares,
    share: &Share,
    viewer: &Viewer,
    mount: &str,
    request: Request,
) -> Response {
    if !share.comments() {
        return page(
            StatusCode::FORBIDDEN,
            "Read only",
            "<p>This link takes no comments.</p>",
        );
    }
    let Ok(bytes) = architect::http::axum::body::to_bytes(request.into_body(), 64 * 1024).await
    else {
        return StatusCode::PAYLOAD_TOO_LARGE.into_response();
    };
    let Ok(form) = serde_urlencoded_form::<CommentForm>(&bytes) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let text = form.text.trim();
    if text.is_empty() || text.chars().count() > MAX_COMMENT || !form.page.starts_with('/') {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let by = match viewer {
        Viewer::Person { email, user_id, .. } => email.clone().unwrap_or_else(|| user_id.clone()),
        Viewer::Anonymous => {
            let name: String = form.name.trim().chars().take(60).collect();
            if name.is_empty() {
                "Someone".to_owned()
            } else {
                format!("{name} (not signed in)")
            }
        }
    };
    let c = ShareComment {
        page: page_key(&form.page),
        by,
        text: text.to_owned(),
        at: now(),
    };
    if let Err(e) = shares.add_comment(share, c).await {
        return page(
            StatusCode::BAD_REQUEST,
            "Not saved",
            &format!("<p>{}</p>", escape(&e.to_string())),
        );
    }
    let back = format!(
        "{}{}#comments",
        mount.trim_end_matches('/'),
        page_key(&form.page)
    );
    Redirect::to(&back).into_response()
}

/// A page's path as comments are keyed: from `/`, ending in `/`.
fn page_key(rest: &str) -> String {
    let path = rest.split(['?', '#']).next().unwrap_or("/");
    let path = path.strip_suffix("index.html").unwrap_or(path);
    if path.ends_with('/') {
        path.to_owned()
    } else {
        format!("{path}/")
    }
}

/// The page with its comments, and the form, before `</main>`.
async fn with_comments(
    shares: &Shares,
    share: &Share,
    rest: &str,
    mount: &str,
    response: Response,
) -> Response {
    let (parts, body) = response.into_parts();
    let Ok(bytes) = architect::http::axum::body::to_bytes(body, 16 * 1024 * 1024).await else {
        return Response::from_parts(parts, Body::empty());
    };
    let html = String::from_utf8_lossy(&bytes);
    let key = page_key(rest);
    let mut section = String::from(COMMENTS_HEAD);
    let comments: Vec<ShareComment> = shares
        .comments(&share.code)
        .into_iter()
        .filter(|c| c.page == key)
        .collect();
    if comments.is_empty() {
        section.push_str("<p class=\"share-comments-none\">No comments yet.</p>");
    }
    for c in comments {
        let _ = write!(
            section,
            "<article class=\"share-comment\"><header><b>{}</b></header><p>{}</p></article>",
            escape(&c.by),
            escape(&c.text).replace('\n', "<br>")
        );
    }
    let _ = write!(
        section,
        "<form method=\"post\" action=\"{}_comment\"><input type=\"hidden\" name=\"page\" value=\"{}\"><input name=\"name\" placeholder=\"Your name\" maxlength=\"60\"><textarea name=\"text\" required maxlength=\"{MAX_COMMENT}\" placeholder=\"Leave a comment\"></textarea><button type=\"submit\">Comment</button></form></section>",
        escape(mount),
        escape(&key)
    );
    let at = html.rfind("</main>").or_else(|| html.rfind("</body>"));
    let out = at.map_or_else(
        || format!("{html}{section}"),
        |i| {
            format!(
                "{}{section}{}",
                html.get(..i).unwrap_or_default(),
                html.get(i..).unwrap_or_default()
            )
        },
    );
    let mut response = Response::from_parts(parts, Body::from(out));
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response
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
        "signin" => start_signin(shares, to, secure).await,
        "signin/wait" => wait_signin(shares, request.headers(), secure).await,
        "signout" => {
            let mut response = Redirect::to(&to).into_response();
            set_cookie(&mut response, SESSION_COOKIE, "", "/", 0, secure);
            response
        }
        _ => page(StatusCode::NOT_FOUND, "Not found", "<p>No such page.</p>"),
    }
}

async fn start_signin(shares: &Shares, to: String, secure: bool) -> Response {
    let (Some(issuer), Some(origin)) = (&shares.0.issuer, &shares.0.issuer_origin) else {
        return page(
            StatusCode::NOT_FOUND,
            "No sign-in",
            "<p>This node signs nobody in.</p>",
        );
    };
    let started = match issuer.client::<auth_proto::AuthServiceClient>().await {
        Ok(auth) => auth
            .start_device_sign_in("architect-share".to_owned())
            .await
            .map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
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
    let polled = match issuer.client::<auth_proto::AuthServiceClient>().await {
        Ok(auth) => auth
            .poll_device_sign_in(device_code, Some("architect-share".to_owned()))
            .await
            .map_err(|e| format!("{e:?}")),
        Err(e) => Err(e),
    };
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

/// Only a path on this node: never another origin.
fn safe_to(to: &str) -> String {
    let ok = to.starts_with('/') && !to.starts_with("//") && !to.contains(['\\', '\r', '\n']);
    if ok { to.to_owned() } else { "/".to_owned() }
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
    response.headers_mut().insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_and_cookies() {
        assert_eq!(page_key("/track/"), "/track/");
        assert_eq!(page_key("/track/index.html"), "/track/");
        assert_eq!(page_key("/"), "/");
        assert_eq!(safe_to("//evil.example"), "/");
        assert_eq!(safe_to("https://evil.example"), "/");
        assert_eq!(safe_to("/abc/x/"), "/abc/x/");
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; architect_session=tok"),
        );
        assert_eq!(cookie(&headers, SESSION_COOKIE).as_deref(), Some("tok"));
        assert_eq!(decode("/a%20b/"), "/a b/");
        assert_eq!(encode("/a b/"), "/a%20b/");
    }
}
