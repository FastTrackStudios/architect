//! A share link opened in the app, as a guest of that link (#629).
//!
//! A link (`https://<node>/s/<code>/…`) arrives from the person
//! (`SHARE_OPEN`: ⌘K, a pasted link, an agent) or from the OS (a
//! universal link, `architect://open?url=…`, a second launch's hand-off,
//! the first launch's arguments: [`incoming`]). It opens in the
//! **Shared** tab, which is never one of the person's vaults: the
//! sharer's node by name, who it is shared with, read only. Its notes are
//! read through the link's guest service (`architect_spaces::ShareGuest`)
//! and drawn as plain blocks; a note whose type an installed extension
//! claims (`Embeds::resolve`) is drawn by it once the note's text is given
//! to guests (`GuestNote::source`), else plain, with "Open in browser".
//! The extension draws it only through its page, on a host that reaches
//! nothing of the person's (`GuestEmbedView`).
//!
//! The rules (`docs/content/guide/deploy.md`, "A link opens Architect"):
//! a link is read with `ShareUrl::parse` (https, or http to this machine;
//! a share link's path); every request goes to its node and nowhere else
//! (no redirect is followed); its code is never logged or shown; a link
//! the OS handed over that points at this machine or a private network
//! (by its name, or by the address it resolves to) opens once the person
//! says so; the account's session is sent only to a node the account
//! already uses for an org of its own ([`trusts`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_extensions::contrib::Claim;
use architect_shell::{icon, use_shell};
use architect_spaces::{BlockKind, GuestLink, GuestNote, PlainBlock, ShareUrl};
use dioxus::prelude::*;
use facet::Facet;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use lucide_dioxus::{ExternalLink, Link2, RefreshCw, X};

use crate::{Lab, Route};

/// The tab a link opens in.
pub const TAB: &str = "shared";

static SHARE_OPEN: ActionMeta = ActionMeta {
    id: "SHARE_OPEN",
    trait_name: "",
    method_name: "",
    display_name: "Open a share link…",
    description: "Opens a share link (`https://<node>/s/<code>/…`, or `architect://open?url=…`) in the Shared tab, read only, as a guest of that link: the notes it shares, as its page shows them. Only https links (http only to this computer) are opened, and nothing is sent anywhere but the link's node.",
    category: "Share",
    group: "Share",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// What `SHARE_OPEN` takes.
#[derive(Debug, Clone, Facet)]
pub struct OpenArgs {
    /// The link.
    pub url: String,
}

/// What `SHARE_OPEN` answers: the node it opens (never the link).
#[derive(Debug, Clone, Facet)]
pub struct OpenOut {
    pub host: String,
}

/// Where a link came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The person: typed, pasted or picked in ⌘K, the menu, a key.
    Person,
    /// The OS: a universal link, the app's scheme, a launch's arguments.
    Os,
    /// `SHARE_OPEN` run by anything but the person (an agent, a script, a
    /// hook): asked about as the OS's links are.
    Agent,
}

/// The node the hosted Architect runs on: its links open straight away.
pub const HOME_HOST: &str = "architect.fasttrackstudio.app";

/// Whether a link opens straight away or asks the person first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Open it.
    No,
    /// It points at this machine or a private network.
    Local,
    /// Its node is neither the hosted Architect nor one of the account's.
    Unknown,
}

// ── The inbox: links from anywhere, opened by the window ─────────────

type Inbox = (
    UnboundedSender<(String, Source)>,
    Mutex<Option<UnboundedReceiver<(String, Source)>>>,
);

fn inbox() -> &'static Inbox {
    static INBOX: OnceLock<Inbox> = OnceLock::new();
    INBOX.get_or_init(|| {
        let (tx, rx) = unbounded();
        (tx, Mutex::new(Some(rx)))
    })
}

/// Open `raw` in the window, from any thread: the OS's links (a scene
/// delegate, a hand-off, a launch's arguments) and `SHARE_OPEN`. Kept
/// until the window takes them.
pub fn incoming(raw: impl Into<String>, from: Source) {
    let _ = inbox().0.unbounded_send((raw.into(), from));
}

/// The share links among a launch's arguments (`architect-desktop
/// https://…/s/<code>/`, or the scheme's `architect://open?url=…`).
#[must_use]
pub fn links_in(args: impl IntoIterator<Item = String>) -> Vec<String> {
    args.into_iter()
        .filter(|a| ShareUrl::parse(a).is_ok())
        .collect()
}

/// The host and port of an org host as the account keeps it
/// (`https://n.example`, `http://127.0.0.1:7422/`, a bare `n.example`):
/// the host lower case; the port it names, or its scheme's (443, 80),
/// or None for a bare host that names neither.
fn host_of(origin: &str) -> (String, Option<u16>) {
    let (scheme, rest) = origin
        .split_once("://")
        .map_or((None, origin), |(s, r)| (Some(s), r));
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let (host, port) = if authority.starts_with('[') {
        let (h, after) = authority.split_once(']').unwrap_or((authority, ""));
        (h.trim_start_matches('['), after.strip_prefix(':'))
    } else {
        authority
            .split_once(':')
            .map_or((authority, None), |(h, p)| (h, Some(p)))
    };
    let by_scheme = || match scheme.map(str::to_ascii_lowercase).as_deref() {
        Some("https") => Some(443),
        Some("http") => Some(80),
        _ => None,
    };
    let port = port.map_or_else(by_scheme, |p| p.parse().ok());
    (host.trim_end_matches('.').to_ascii_lowercase(), port)
}

/// The port a link's requests go to.
fn port_of(url: &ShareUrl) -> Option<u16> {
    url::Url::parse(&url.origin()).ok()?.port_or_known_default()
}

/// Whether a link's node is one the app knows, on its own port.
///
/// The hosted Architect on 443, or a node the account uses for one of
/// its orgs (`known`, as [`trusts`] takes them) on the port that entry
/// names (its own, or its scheme's): another port of the same name may
/// be another server.
#[must_use]
pub fn known_host(url: &ShareUrl, known: &[String]) -> bool {
    let host = url
        .host()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let port = port_of(url);
    (host == HOME_HOST && port == Some(443))
        || known.iter().any(|k| {
            let (h, p) = host_of(k);
            h == host && p.is_none_or(|p| Some(p) == port)
        })
}

/// Whether a link asks the person first.
///
/// One the person opened does not, as a browser's address bar does not.
/// One the OS handed over or an
/// agent asked for does when it points at this machine or a private
/// network ([`Ask::Local`], first) or at a node the app does not know
/// ([`Ask::Unknown`]: nothing is fetched until the person says yes).
#[must_use]
pub fn asks_first(url: &ShareUrl, from: Source, known: &[String]) -> Ask {
    if from == Source::Person {
        Ask::No
    } else if url.is_local() {
        Ask::Local
    } else if known_host(url, known) {
        Ask::No
    } else {
        Ask::Unknown
    }
}

/// Whether the account may send its session to `origin`.
///
/// Only to a node it already uses for an org of its own (`org_hosts`, and
/// the one `ARCHITECT_ORG_HOST` names), never to a node a link merely
/// points at: a node that sees a session can act as its person.
#[must_use]
pub fn trusts(org_hosts: &[String], origin: &str) -> bool {
    let origin = origin.trim_end_matches('/');
    org_hosts
        .iter()
        .any(|h| h.trim_end_matches('/').eq_ignore_ascii_case(origin))
}

// ── Why a link does not open ─────────────────────────────────────────

/// Why a link (or a note of it) is not shown. Never carries the link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Why {
    /// A link the app does not open; the words say why.
    Refused(String),
    /// Expired, revoked, never there, or not on this link.
    Gone,
    /// For signed-in people.
    SignIn,
    /// Not shared with this account.
    Denied,
    /// Being prepared on the node.
    Building,
    /// The node did not answer.
    Unreachable,
    /// The node answered something that does not read.
    Broken,
}

impl Why {
    /// A title and a line for the person.
    #[must_use]
    pub fn words(&self, host: &str) -> (String, String) {
        let (title, line) = match self {
            Self::Refused(why) => ("Not opened", why.clone()),
            Self::Gone => (
                "This link doesn't work",
                "It has been turned off, it has expired, or it was never there.".to_owned(),
            ),
            Self::SignIn => (
                "For signed-in people",
                format!(
                    "This link on {host} is for people signed in there. Open it in your browser to sign in."
                ),
            ),
            Self::Denied => (
                "Not shared with you",
                "This link was not shared with the account you are signed in as.".to_owned(),
            ),
            Self::Building => (
                "Being prepared",
                format!("{host} is preparing this link. Try again in a moment."),
            ),
            Self::Unreachable => ("Can't reach the node", format!("{host} did not answer.")),
            Self::Broken => (
                "Can't read this link",
                format!("{host} answered with something this Architect can't read."),
            ),
        };
        (title.to_owned(), line)
    }

    /// Whether the browser may do better (its sign-in, the page).
    #[must_use]
    pub const fn browser_helps(&self) -> bool {
        matches!(
            self,
            Self::SignIn | Self::Denied | Self::Unreachable | Self::Broken
        )
    }
}

// ── The session: the link's guest service, on its node alone ─────────

#[cfg(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "windows",
    target_os = "ios"
))]
mod net {
    use std::net::SocketAddr;
    use std::time::Duration;

    use architect::ClientError;
    use architect_spaces::guest::ShareGuestHttpClient;
    use architect_spaces::{GuestFault, GuestLink, GuestNote, ShareUrl};
    use base64::Engine as _;

    use super::Why;

    /// The most a guest reads of one image.
    const MAX_IMAGE: usize = 16 << 20;
    /// How long one request may take.
    const TIMEOUT: Duration = Duration::from_secs(20);
    /// How often a link is read again with no news of it.
    const EVERY: Duration = Duration::from_secs(300);
    /// How long to wait after the news stream ended or failed.
    const AGAIN: Duration = Duration::from_secs(30);

    fn runtime() -> Result<&'static tokio::runtime::Runtime, Why> {
        static RT: std::sync::OnceLock<Option<tokio::runtime::Runtime>> =
            std::sync::OnceLock::new();
        RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_name("share-guest")
                .enable_all()
                .build()
                .ok()
        })
        .as_ref()
        .ok_or(Why::Unreachable)
    }

    async fn on_runtime<T: Send + 'static>(
        work: impl Future<Output = T> + Send + 'static,
    ) -> Result<T, Why> {
        runtime()?.spawn(work).await.map_err(|_| Why::Unreachable)
    }

    /// A client that follows no redirect: a node's answer is its own.
    /// With `pinned`, it connects to those addresses for `host` and never
    /// asks the DNS again, so a name checked as public stays so (no DNS
    /// rebinding between the check and the requests).
    fn http(timeout: Option<Duration>, pinned: Option<(&str, &[SocketAddr])>) -> reqwest::Client {
        let builder = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none());
        let builder = match timeout {
            Some(t) => builder.timeout(t),
            None => builder,
        };
        let builder = match pinned {
            Some((host, addrs)) => builder.resolve_to_addrs(host, addrs),
            None => builder,
        };
        builder.build().unwrap_or_default()
    }

    /// A link's guest session: its node, the client, and the session it
    /// sends (empty unless the node is trusted).
    #[derive(Clone)]
    pub struct Session {
        url: ShareUrl,
        http: reqwest::Client,
        token: String,
        /// The addresses its host was checked at ([`Self::pinned`]).
        pinned: Option<Vec<SocketAddr>>,
    }

    /// What a link's host resolves to.
    pub enum Resolved {
        /// This machine or a private network, at one address at least.
        Local,
        /// Public addresses only, to connect to and nothing else.
        Public(Vec<SocketAddr>),
        /// Nothing (the request then fails as unreachable).
        Unknown,
    }

    const fn why(e: &ClientError<GuestFault>) -> Why {
        match e {
            ClientError::App(GuestFault::Gone) => Why::Gone,
            ClientError::App(GuestFault::SignIn) => Why::SignIn,
            ClientError::App(GuestFault::Denied | GuestFault::Held(_)) => Why::Denied,
            ClientError::App(GuestFault::Building) => Why::Building,
            ClientError::Connect(_)
            | ClientError::Transport {
                retryable: true, ..
            } => Why::Unreachable,
            ClientError::App(GuestFault::Transport(_)) | ClientError::Transport { .. } => {
                Why::Broken
            }
        }
    }

    impl Session {
        pub fn new(url: ShareUrl, token: String) -> Self {
            Self {
                url,
                http: http(Some(TIMEOUT), None),
                token,
                pinned: None,
            }
        }

        fn bare_host(&self) -> String {
            self.url
                .host()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned()
        }

        /// This session, connecting only to `addrs` (those its host was
        /// checked at).
        #[must_use]
        pub fn pinned(mut self, addrs: Vec<SocketAddr>) -> Self {
            let host = self.bare_host();
            self.http = http(Some(TIMEOUT), Some((&host, &addrs)));
            self.pinned = Some(addrs);
            self
        }

        fn client(&self) -> ShareGuestHttpClient {
            ShareGuestHttpClient::new(
                architect::http::HttpClient::new(self.url.origin()).with_reqwest(self.http.clone()),
            )
        }

        pub async fn link(&self) -> Result<GuestLink, Why> {
            let (client, code, token) = (
                self.client(),
                self.url.code().to_owned(),
                self.token.clone(),
            );
            on_runtime(async move { client.link(code, token).await })
                .await?
                .map_err(|e| why(&e))
        }

        pub async fn read(&self, path: String) -> Result<GuestNote, Why> {
            let (client, code, token) = (
                self.client(),
                self.url.code().to_owned(),
                self.token.clone(),
            );
            on_runtime(async move { client.read(code, token, path).await })
                .await?
                .map_err(|e| why(&e))
        }

        /// An image of the link (`src`, a path below it) as a `data:`
        /// URL; None when it is not one of the link's, not an image, too
        /// big, or not there.
        pub async fn image(&self, src: String) -> Option<String> {
            let url = super::image_url(&self.url, &src)?;
            let http = self.http.clone();
            on_runtime(async move {
                let mut res = http.get(url).send().await.ok()?;
                if !res.status().is_success() {
                    return None;
                }
                let kind = res
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .filter(|k| {
                        ["image/png", "image/jpeg", "image/gif", "image/webp"]
                            .iter()
                            .any(|ok| k.starts_with(ok))
                    })?
                    .to_owned();
                let mut bytes = Vec::new();
                while let Some(chunk) = res.chunk().await.ok()? {
                    if bytes.len().saturating_add(chunk.len()) > MAX_IMAGE {
                        return None;
                    }
                    bytes.extend_from_slice(&chunk);
                }
                let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
                Some(format!("data:{kind};base64,{data}"))
            })
            .await
            .ok()
            .flatten()
        }

        /// What the link's host resolves to: this machine or a private
        /// network (a public-looking name can hide one), or the public
        /// addresses to pin the session to ([`Self::pinned`]).
        pub async fn resolve(&self) -> Resolved {
            let host = self.bare_host();
            let port = self
                .url
                .host_label()
                .rsplit_once(':')
                .and_then(|(_, p)| p.parse().ok())
                .filter(|_| !self.url.host_label().ends_with(']'))
                .unwrap_or(443);
            on_runtime(async move {
                let Ok(found) = tokio::net::lookup_host((host.as_str(), port)).await else {
                    return Resolved::Unknown;
                };
                let found: Vec<SocketAddr> = found.collect();
                if found.is_empty() {
                    Resolved::Unknown
                } else if found
                    .iter()
                    .any(|a| architect_spaces::guest::is_private_address(a.ip()))
                {
                    Resolved::Local
                } else {
                    Resolved::Public(found)
                }
            })
            .await
            .unwrap_or(Resolved::Unknown)
        }

        /// Wait for news of the link: an event on its `_live` stream (a
        /// rebuild, a revoke), or [`EVERY`] with none.
        pub async fn changed(&self) {
            let host = self.bare_host();
            let pinned = self.pinned.as_deref().map(|a| (host.as_str(), a));
            let (http, url) = (http(None, pinned), format!("{}_live", self.url.link()));
            let _ = on_runtime(async move {
                let wait = async {
                    let res = http.get(url).send().await;
                    let Ok(mut res) = res.map_err(drop).and_then(|r| {
                        if r.status().is_success() {
                            Ok(r)
                        } else {
                            Err(())
                        }
                    }) else {
                        tokio::time::sleep(AGAIN).await;
                        return;
                    };
                    loop {
                        match res.chunk().await {
                            Ok(Some(chunk)) if chunk.windows(5).any(|w| w == b"data:") => return,
                            Ok(Some(_)) => {}
                            _ => {
                                tokio::time::sleep(AGAIN).await;
                                return;
                            }
                        }
                    }
                };
                let _ = tokio::time::timeout(EVERY, wait).await;
            })
            .await;
        }
    }
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "linux",
    target_os = "windows",
    target_os = "ios"
)))]
mod net {
    use architect_spaces::{GuestLink, GuestNote, ShareUrl};

    use super::Why;

    #[derive(Clone)]
    pub struct Session;

    pub enum Resolved {
        Local,
        #[allow(dead_code)]
        Public(Vec<std::net::SocketAddr>),
        #[allow(dead_code)]
        Unknown,
    }

    impl Session {
        pub const fn new(_url: ShareUrl, _token: String) -> Self {
            Self
        }
        pub async fn link(&self) -> Result<GuestLink, Why> {
            Err(Why::Unreachable)
        }
        pub async fn read(&self, _path: String) -> Result<GuestNote, Why> {
            Err(Why::Unreachable)
        }
        pub async fn image(&self, _src: String) -> Option<String> {
            None
        }
        #[must_use]
        pub fn pinned(self, _addrs: Vec<std::net::SocketAddr>) -> Self {
            self
        }
        pub async fn resolve(&self) -> Resolved {
            Resolved::Local
        }
        pub async fn changed(&self) {
            std::future::pending::<()>().await;
        }
    }
}

use net::{Resolved, Session};

/// Where to fetch an image of the link (`src`, as its note gave it).
///
/// `src` parsed against the link's origin and normalised as a browser
/// would before fetching it. `None` unless it is on that origin, below the
/// link (`/s/<code>/…`), names a file there, and carries no query,
/// fragment, credentials, empty segment, or encoded dot, slash, backslash
/// or NUL a server could decode into another path (as #673's
/// `guest_file_url` in `apps/web/src/view_files.rs`).
#[must_use]
pub fn image_url(link: &ShareUrl, src: &str) -> Option<String> {
    let base = url::Url::parse(&link.origin()).ok()?;
    let parsed = base.join(src).ok()?;
    let path = parsed.path();
    let rest = path.strip_prefix(&link.mount())?;
    let lower = path.to_ascii_lowercase();
    let plain = parsed.origin() == base.origin()
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && !rest.is_empty()
        && !rest.ends_with('/')
        && !rest.split('/').any(str::is_empty)
        && !["%2e", "%2f", "%5c", "%00"]
            .iter()
            .any(|e| lower.contains(e));
    plain.then(|| parsed.to_string())
}

// ── The window's state ───────────────────────────────────────────────

/// Where an open link is.
#[derive(Clone, PartialEq, Eq)]
pub enum Phase {
    /// The OS or an agent handed over a link to this machine, a private
    /// network or a node the app does not know: the person says whether
    /// to open it.
    Asking(Ask),
    Loading,
    Open(GuestLink),
    Failed(Why),
}

/// A note of the link, as it is read.
#[derive(Clone, PartialEq, Eq)]
pub enum NoteView {
    Loading(String),
    Ready(GuestNote),
    Failed(String, Why),
}

impl NoteView {
    fn path(&self) -> &str {
        match self {
            Self::Ready(n) => &n.path,
            Self::Loading(p) | Self::Failed(p, _) => p,
        }
    }
}

/// The link open in the Shared tab.
#[derive(Clone, PartialEq, Eq)]
pub struct Opened {
    /// None: a link the app refused (why, in `phase`).
    pub url: Option<ShareUrl>,
    pub phase: Phase,
    pub note: Option<NoteView>,
    /// Where the link came from: kept for as long as it is open, so
    /// reading it again ("Try again") asks what its first opening asked.
    pub from: Source,
    /// The person said yes to [`Ask::Local`]: this machine or a private
    /// network is where they expect it to be.
    pub local_ok: bool,
}

impl Opened {
    /// Whether opening it (again) looks up its host first: unless the
    /// person opened it or said yes to a private network, its host is
    /// resolved, a private address asks, and the requests go to the
    /// public addresses it found alone. A yes to an unknown node is not
    /// a yes to a private network, and a "Try again" checks again: the
    /// DNS may answer otherwise the second time.
    #[must_use]
    pub fn checks_address(&self) -> bool {
        self.from != Source::Person && !self.local_ok
    }

    /// The person said yes to (or asked again for) this link: whether a
    /// private network is now allowed.
    #[must_use]
    pub fn local_ok_after_yes(&self) -> bool {
        self.local_ok || self.phase == Phase::Asking(Ask::Local)
    }
}

/// Where an opening goes once its host is looked up: on to the addresses
/// to pin it to, or to a phase that stops it. A lookup that found nothing
/// (it failed, or answered no address) stops it as unreachable: the
/// request's own lookup could answer otherwise, a private address the
/// check never saw.
fn after_lookup(resolved: Resolved) -> Result<Vec<std::net::SocketAddr>, Phase> {
    match resolved {
        Resolved::Local => Err(Phase::Asking(Ask::Local)),
        Resolved::Public(addrs) => Ok(addrs),
        Resolved::Unknown => Err(Phase::Failed(Why::Unreachable)),
    }
}

/// The session to send to a node's origin: empty unless it is trusted.
type Trust = Arc<dyn Fn(&str) -> String>;
/// The nodes the account uses for its orgs ([`trusts`], [`known_host`]).
type Known = Arc<dyn Fn() -> Vec<String>>;

/// The Shared tab's state and what opens a link in it. Cheap to copy.
#[derive(Clone, Copy)]
pub struct Guest {
    open: Signal<Option<Opened>>,
    /// Images of the open link by `src`, as `data:` URLs.
    images: Signal<HashMap<String, String>>,
    /// Which opening is current: a newer one ends the older one's work.
    generation: Signal<u64>,
    trust: Signal<Trust>,
    known: Signal<Known>,
    /// The open link's session as its first read made it (pinned to the
    /// addresses it was checked at): a note shown later goes the same way.
    current: Signal<Option<Session>>,
    lab: Lab,
}

// As a component's prop: the same signals are the same state.
impl PartialEq for Guest {
    fn eq(&self, other: &Self) -> bool {
        self.open == other.open && self.images == other.images
    }
}

impl Guest {
    /// A new opening: the older one's work ends.
    fn bump(&mut self) {
        let next = self.generation.peek().wrapping_add(1);
        self.generation.set(next);
    }

    /// Whether a link is open (the Shared tab shows).
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.read().is_some()
    }

    /// What is open.
    #[must_use]
    pub fn opened(&self) -> Option<Opened> {
        self.open.read().clone()
    }

    /// Whether `url` (from `from`) asks the person first.
    fn asks(&self, url: &ShareUrl, from: Source) -> Ask {
        let known = (self.known.peek())();
        asks_first(url, from, &known)
    }

    /// Open `raw` (from `from`) in the Shared tab.
    pub fn open(mut self, raw: &str, from: Source) {
        self.images.write().clear();
        self.current.set(None);
        match ShareUrl::parse(raw) {
            Err(why) => {
                self.bump();
                self.open.set(Some(Opened {
                    url: None,
                    phase: Phase::Failed(Why::Refused(why.to_string())),
                    note: None,
                    from,
                    local_ok: false,
                }));
            }
            Ok(url) => match self.asks(&url, from) {
                Ask::No => self.start(url, from, false),
                ask => {
                    self.bump();
                    self.open.set(Some(Opened {
                        url: Some(url),
                        phase: Phase::Asking(ask),
                        note: None,
                        from,
                        local_ok: false,
                    }));
                }
            },
        }
        self.lab.jump(Route::Tab(TAB));
    }

    /// Which opening is shown now: what a button drawn for it carries, so
    /// a press acts on that opening and no other ([`Self::retry`]).
    #[must_use]
    pub fn generation(&self) -> u64 {
        *self.generation.peek()
    }

    /// The person said yes to a link that asked first; or read the open
    /// link again. `at` is the opening the button was drawn for
    /// ([`Self::generation`]): a link opened since (an agent's, the OS's)
    /// is never what the press says yes to.
    pub fn retry(self, at: u64) {
        if *self.generation.peek() != at {
            return;
        }
        let Some(opened) = self.opened() else {
            return;
        };
        if let Some(url) = opened.url.clone() {
            // A yes to an unknown node is not a yes to a private network:
            // the address it resolves to is still asked about, on every
            // reading ([`Opened::checks_address`]).
            self.start(url, opened.from, opened.local_ok_after_yes());
        }
    }

    /// Close the link: nothing of it is kept.
    pub fn close(mut self) {
        self.bump();
        self.open.set(None);
        self.current.set(None);
        self.images.write().clear();
        self.lab.jump(Route::Tab("vault"));
    }

    fn session(&self, url: ShareUrl) -> Session {
        let token = (self.trust.peek())(&url.origin());
        Session::new(url, token)
    }

    /// Open `url` (from `from`; `local_ok`: the person said yes to a
    /// private network): read the link and its first note, then keep
    /// both current until the link closes, goes or another opens.
    fn start(mut self, url: ShareUrl, from: Source, local_ok: bool) {
        self.bump();
        self.current.set(None);
        let at = *self.generation.peek();
        let session = self.session(url.clone());
        let opened = Opened {
            url: Some(url),
            phase: Phase::Loading,
            note: None,
            from,
            local_ok,
        };
        let check = opened.checks_address();
        self.open.set(Some(opened));
        let mut open = self.open;
        let generation = self.generation;
        let current = move || *generation.peek() == at;
        dioxus::core::spawn_forever(async move {
            // A name that resolves to a private network asks first too;
            // one that resolves to public addresses is read from those
            // alone, so a second answer from the DNS can't move it.
            let mut session = session;
            if check {
                match after_lookup(session.resolve().await) {
                    Ok(addrs) => session = session.pinned(addrs),
                    Err(phase) => {
                        if current() {
                            if let Some(o) = open.write().as_mut() {
                                o.phase = phase;
                            }
                        }
                        return;
                    }
                }
            }
            if !current() {
                return;
            }
            let mut kept = self.current;
            kept.set(Some(session.clone()));
            let mut first = true;
            loop {
                let link = session.link().await;
                if !current() {
                    return;
                }
                // Revoked, expired, or no longer this account's to read:
                // nothing of it is kept.
                let gone = matches!(link, Err(Why::Gone));
                let shut = gone || matches!(link, Err(Why::SignIn | Why::Denied));
                let first_note = link
                    .as_ref()
                    .ok()
                    .and_then(|l| l.notes.iter().find(|n| n.page.is_some()))
                    .map(|n| n.path.clone());
                if let Some(o) = open.write().as_mut() {
                    o.phase = match link {
                        Ok(link) => Phase::Open(link),
                        Err(why) => Phase::Failed(why),
                    };
                    if shut {
                        o.note = None;
                    }
                }
                if shut {
                    self.images.write().clear();
                }
                if gone {
                    return;
                }
                // The note on show, read again (the first one, at first).
                let showing = open
                    .peek()
                    .as_ref()
                    .and_then(|o| o.note.as_ref().map(|n| n.path().to_owned()));
                let next = showing.or_else(|| first_note.filter(|_| first));
                if let Some(path) = next {
                    self.load_note(&session, path, at).await;
                }
                first = false;
                session.changed().await;
                if !current() {
                    return;
                }
            }
        });
    }

    // On the window's executor, as its signals are.
    #[allow(clippy::future_not_send)]
    async fn load_note(mut self, session: &Session, path: String, at: u64) {
        let read = session.read(path.clone()).await;
        if *self.generation.peek() != at {
            return;
        }
        let images: Vec<String> = read
            .as_ref()
            .map(|n| {
                n.blocks
                    .iter()
                    .filter(|b| b.kind == BlockKind::Image)
                    .filter_map(|b| b.src.clone())
                    .collect()
            })
            .unwrap_or_default();
        if let Some(o) = self.open.write().as_mut() {
            o.note = Some(match read {
                Ok(note) => NoteView::Ready(note),
                Err(why) => NoteView::Failed(path, why),
            });
        }
        if matches!(
            self.open.peek().as_ref().and_then(|o| o.note.as_ref()),
            Some(NoteView::Failed(_, Why::SignIn | Why::Denied | Why::Gone))
        ) {
            // The note's words went with the failure; its images go too.
            self.images.write().clear();
            return;
        }
        for src in images {
            if self.images.peek().contains_key(&src) {
                continue;
            }
            if let Some(data) = session.image(src.clone()).await {
                if *self.generation.peek() == at {
                    self.images.write().insert(src, data);
                }
            }
        }
    }

    /// Show the note at `path` of the open link.
    pub fn show(mut self, path: String) {
        let Some(Opened { url: Some(_), .. }) = self.opened() else {
            return;
        };
        if let Some(o) = self.open.write().as_mut() {
            o.note = Some(NoteView::Loading(path.clone()));
        }
        let at = *self.generation.peek();
        // Only the open link's own session, pinned when it was read: a
        // click from a view of a link since replaced (or still asking
        // first) reads nothing.
        let Some(session) = self.current.peek().clone() else {
            return;
        };
        dioxus::core::spawn_forever(async move {
            self.load_note(&session, path, at).await;
        });
    }
}

/// The Shared tab's state, `SHARE_OPEN` on `list`, and the window taking
/// the links handed to it ([`incoming`]).
///
/// `known` answers the nodes the
/// account uses for its orgs; `trust` the session to send to a node's
/// origin (empty: none).
pub fn use_guest(
    list: &Arc<ActionList>,
    lab: Lab,
    known: impl Fn() -> Vec<String> + 'static,
    trust: impl Fn(&str) -> String + 'static,
) -> Guest {
    let open = use_signal(|| None);
    let images = use_signal(HashMap::new);
    let generation = use_signal(|| 0_u64);
    let trust = use_signal(move || -> Trust { Arc::new(trust) });
    let known = use_signal(move || -> Known { Arc::new(known) });
    let current = use_signal(|| None);
    let guest = Guest {
        open,
        images,
        generation,
        trust,
        known,
        current,
        lab,
    };
    use_hook(|| {
        let _ = list.register_with(&SHARE_OPEN, Effect::ReadOnly, |caller, args: OpenArgs| {
            let url = ShareUrl::parse(&args.url).map_err(|why| why.to_string())?;
            incoming(args.url.trim(), source_of(caller));
            Ok(OpenOut {
                host: url.host_label(),
            })
        });
        // The first launch's own links (a second launch hands its over).
        for link in links_in(std::env::args().skip(1)) {
            incoming(link, Source::Os);
        }
        // And its deep links into actions (#186).
        for arg in std::env::args().skip(1) {
            let _ = crate::links::incoming(arg);
        }
    });
    use_hook(move || {
        let taken = inbox()
            .1
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(mut links) = taken {
            dioxus::core::spawn_forever(async move {
                use futures_util::StreamExt as _;
                while let Some((raw, from)) = links.next().await {
                    guest.open(&raw, from);
                }
            });
        }
    });
    guest
}

/// Who ran `SHARE_OPEN`: the person only from the app's own doors (⌘K,
/// the menu, a key, a toolbar), which say so (`Commands::run_from`), with
/// no principal; anything else (an agent, a script, an extension's page,
/// a hook, a peer, a run that says nothing of where it came from) is
/// asked about as the OS's links are.
fn source_of(caller: &architect::action::ActionCaller) -> Source {
    use architect::action::InvocationSource as From;
    let by_person = caller.principal().is_none()
        && caller.invocation().is_some_and(|i| {
            matches!(
                i.source,
                From::Palette | From::Menu | From::Key | From::ContextMenu | From::Toolbar
            )
        });
    if by_person {
        Source::Person
    } else {
        Source::Agent
    }
}

/// The Shared tab in the shell's tabs.
///
/// On a phone it is kept on the bar and takes the fourth place (`architect_shell::phone_tabs`): the
/// person's last favourite, still in ⌘K and the sidebar, gives way while
/// the link is open, so the link is one tap away after leaving it.
#[must_use]
pub fn tab() -> architect_shell::Tab {
    architect_shell::Tab::new(TAB, "Shared", icon!(Link2)).kept()
}

// ── The Shared tab ───────────────────────────────────────────────────

fn open_in_browser(url: &str) {
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "explorer"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(opener).arg(url).spawn();
    }
    #[cfg(target_os = "ios")]
    {
        use objc2_foundation::{NSDictionary, NSString, NSURL};
        // Buttons run on the main thread; elsewhere there is no app to ask.
        let Some(mtm) = objc2::MainThreadMarker::new() else {
            return;
        };
        let Some(url) = NSURL::URLWithString(&NSString::from_str(url)) else {
            return;
        };
        let app = objc2_ui_kit::UIApplication::sharedApplication(mtm);
        let options = NSDictionary::new();
        // SAFETY: the options are an empty dictionary of the declared key
        // and value types; no completion handler is passed.
        unsafe { app.openURL_options_completionHandler(&url, &options, None) };
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows",
        target_os = "ios"
    )))]
    {
        let _ = url;
    }
}

/// Who a link is for, as its header says it.
fn for_whom(audience: &str) -> &'static str {
    match audience {
        "org" => "Shared with an organization's members",
        "invited" => "Shared with invited people",
        "public" => "Shared publicly",
        _ => "Shared with a link",
    }
}

/// A note's page below its link, as a URL on its node.
fn page_url(url: &ShareUrl, page: &str) -> String {
    url.page_url(
        page.strip_prefix(url.mount().trim_end_matches('/'))
            .unwrap_or("/"),
    )
}

fn button_style(t: &architect_shell::ShellTheme) -> String {
    format!(
        "display: inline-flex; align-items: center; gap: 6px; padding: 6px 12px; border-radius: 8px; \
         border: 1px solid {}; background: {}; color: {}; font-size: 13px; cursor: pointer;",
        t.rule, t.pill, t.ink
    )
}

/// The Shared tab: the open link, never one of the person's vaults.
#[component]
pub fn GuestPage(guest: Guest) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    // On a touch screen the header sits below the status bar and the
    // notch, as a page's bar does (`architect_shell::PageBar`).
    let ins = shell.screen.insets;
    let (pad_top, pad_left, pad_right) = if shell.desk() {
        (14.0, 16.0, 16.0)
    } else {
        (14.0 + ins.top, 16.0 + ins.left, 16.0 + ins.right)
    };
    let Some(opened) = guest.opened() else {
        return rsx! {
            div { style: "padding: 32px; color: {t.ink_3}; font-size: 14px;",
                "No share link is open. ⌘K, then Open a share link…"
            }
        };
    };
    let host = opened
        .url
        .as_ref()
        .map(ShareUrl::host_label)
        .unwrap_or_default();
    let browser = opened.url.as_ref().map(|u| match &opened.note {
        Some(NoteView::Ready(n)) => page_url(u, &n.page),
        _ => u.page_url(u.rest()),
    });
    // A link that doesn't work has no page to open either.
    let header_browser = browser
        .clone()
        .filter(|_| !matches!(opened.phase, Phase::Failed(Why::Gone | Why::Refused(_))));
    let (title, whom) = match &opened.phase {
        Phase::Open(link) if !link.title.is_empty() => {
            (link.title.clone(), for_whom(&link.audience))
        }
        Phase::Open(link) => ("Shared notes".to_owned(), for_whom(&link.audience)),
        _ => ("Share link".to_owned(), "Shared with a link"),
    };
    let from = if host.is_empty() {
        format!("{whom} · Read only")
    } else {
        format!("{host} · {whom} · Read only")
    };
    let button = button_style(&t);
    // The opening these buttons are drawn for ([`Guest::retry`]).
    let drawn_at = guest.generation();
    rsx! {
        div { class: "guest-page",
            style: "display: flex; flex-direction: column; height: 100%; min-height: 0; background: {t.sheet}; color: {t.ink};",
            div { class: "guest-head",
                style: "display: flex; flex-wrap: wrap; align-items: center; gap: 12px; padding: {pad_top}px {pad_right}px 14px {pad_left}px; border-bottom: 1px solid {t.rule};",
                div { style: "display: flex; flex-direction: column; gap: 2px; flex: 1 1 200px; min-width: 0;",
                    div { style: "display: flex; align-items: center; gap: 8px; font-size: 17px; font-weight: 700;",
                        Link2 { size: 16, color: t.ink_3.to_string() }
                        span { "{title}" }
                    }
                    div { class: "guest-from", style: "font-size: 12px; color: {t.ink_3};", "{from}" }
                }
                if let Some(url) = header_browser {
                    button { style: "{button}", onclick: move |_| open_in_browser(&url),
                        ExternalLink { size: 14, color: t.ink_2.to_string() }
                        "Open in browser"
                    }
                }
                button { style: "{button}", title: "Close this link: nothing of it is kept",
                    onclick: move |_| guest.close(),
                    X { size: 14, color: t.ink_2.to_string() }
                    "Close"
                }
            }
            match opened.phase.clone() {
                Phase::Asking(ask) => rsx! {
                    Notice {
                        title: if ask == Ask::Unknown { format!("Open a shared link from {host}?") } else { "Open a link to your own network?".to_owned() },
                        line: if ask == Ask::Unknown {
                            format!("{host} is not a node you use. Nothing is read from it until you open the link. Open it only if you expected it.")
                        } else {
                            format!("This link points at this computer or one on your own network: {host}. Open it only if you expected it.")
                        },
                        div { style: "display: flex; gap: 8px;",
                            button { style: "{button}", onclick: move |_| guest.retry(drawn_at), "Open it" }
                            button { style: "{button}", onclick: move |_| guest.close(), "Cancel" }
                        }
                    }
                },
                Phase::Loading => rsx! {
                    Notice { title: "Opening…", line: format!("Reading the link from {host}.") }
                },
                Phase::Failed(why) => {
                    let (title, line) = why.words(&host);
                    let helps = why.browser_helps() && browser.is_some();
                    let url = browser.unwrap_or_default();
                    let retry = opened.url.is_some();
                    rsx! {
                        Notice { title, line,
                            div { style: "display: flex; gap: 8px;",
                                if retry {
                                    button { style: "{button}", onclick: move |_| guest.retry(drawn_at),
                                        RefreshCw { size: 14, color: t.ink_2.to_string() }
                                        "Try again"
                                    }
                                }
                                if helps {
                                    button { style: "{button}", onclick: move |_| open_in_browser(&url),
                                        ExternalLink { size: 14, color: t.ink_2.to_string() }
                                        "Open in browser"
                                    }
                                }
                            }
                        }
                    }
                }
                Phase::Open(link) => rsx! {
                    LinkView { guest, link, note: opened.note.clone(), url: opened.url.clone() }
                },
            }
        }
    }
}

#[component]
fn Notice(title: String, line: String, children: Element) -> Element {
    let t = use_shell().theme;
    rsx! {
        div { class: "guest-notice",
            style: "display: flex; flex-direction: column; gap: 10px; margin: 32px auto; padding: 0 16px; max-width: 480px;",
            div { style: "font-size: 18px; font-weight: 700; color: {t.ink};", "{title}" }
            div { style: "font-size: 14px; line-height: 1.5; color: {t.ink_2};", "{line}" }
            {children}
        }
    }
}

/// The link's notes and the one on show: side by side, stacked when
/// narrow.
#[component]
fn LinkView(
    guest: Guest,
    link: GuestLink,
    note: Option<NoteView>,
    url: Option<ShareUrl>,
) -> Element {
    let shell = use_shell();
    let t = shell.theme;
    let narrow = shell.screen.width < 700.0;
    let showing = note.as_ref().map(|n| n.path().to_owned());
    let direction = if narrow { "column" } else { "row" };
    let list_style = if narrow {
        format!(
            "display: flex; gap: 6px; overflow-x: auto; padding: 10px 16px; border-bottom: 1px solid {};",
            t.rule
        )
    } else {
        format!(
            "display: flex; flex-direction: column; gap: 2px; width: 240px; flex: none; overflow-y: auto; padding: 10px 8px; border-right: 1px solid {};",
            t.rule
        )
    };
    let host = url.as_ref().map(ShareUrl::host_label).unwrap_or_default();
    rsx! {
        div { style: "display: flex; flex-direction: {direction}; flex: 1 1 0; min-height: 0;",
            div { class: "guest-notes", style: "{list_style}",
                for n in link.notes.iter().filter(|n| n.page.is_some()) {
                    {
                        let path = n.path.clone();
                        let on = showing.as_deref() == Some(n.path.as_str());
                        let bg = if on { t.active.to_string() } else { "transparent".to_owned() };
                        rsx! {
                            button { key: "{n.path}", class: "guest-note-row",
                                style: "text-align: left; padding: 6px 10px; border-radius: 6px; border: 0; background: {bg}; color: {t.ink}; font-size: 13px; cursor: pointer; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; flex: none;",
                                onclick: move |_| guest.show(path.clone()),
                                "{n.title}"
                            }
                        }
                    }
                }
                if link.notes.is_empty() {
                    div { style: "padding: 8px; color: {t.ink_3}; font-size: 13px;", "Nothing is shared on this link yet." }
                }
            }
            div { class: "guest-note", style: "flex: 1 1 0; min-width: 0; min-height: 0; overflow-y: auto; padding: 20px 24px;",
                match note {
                    None => rsx! {},
                    Some(NoteView::Loading(_)) => rsx! {
                        div { style: "color: {t.ink_3}; font-size: 13px;", "Reading…" }
                    },
                    Some(NoteView::Failed(_, why)) => {
                        let (title, line) = why.words(&host);
                        rsx! { Notice { title, line } }
                    }
                    Some(NoteView::Ready(n)) => rsx! { GuestNoteView { guest, note: n, url } },
                }
            }
        }
    }
}

/// A note: drawn by the extension that claims its type when it may be
/// (its text given to guests), else its page's words as plain blocks.
#[component]
fn GuestNoteView(guest: Guest, note: GuestNote, url: Option<ShareUrl>) -> Element {
    let t = use_shell().theme;
    let ui = try_consume_context::<architect_extensions::interface::ExtensionUi>();
    let claim = note.kind.clone().map(Claim::NoteType);
    let claimed = match (&ui, &claim) {
        (Some(ui), Some(claim)) => ui.host().embeds().resolve(claim).is_some(),
        _ => false,
    };
    let page = url.as_ref().map(|u| page_url(u, &note.page));
    let plain = rsx! {
        PlainBlocks { guest, blocks: note.blocks.clone(), page: page.clone() }
    };
    let drawn = match (&claim, &note.source) {
        (Some(claim), Some(source)) if claimed => Some((claim.clone(), source.clone())),
        _ => None,
    };
    let undrawn_kind = note.kind.clone().filter(|_| drawn.is_none());
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 12px; max-width: 680px;",
            div { style: "font-size: 24px; font-weight: 700; color: {t.ink};", "{note.title}" }
            if !note.tags.is_empty() {
                div { style: "display: flex; gap: 6px; flex-wrap: wrap;",
                    for tag in note.tags.iter() {
                        span { key: "{tag}",
                            style: "font-size: 12px; padding: 2px 8px; border-radius: 999px; background: {t.pill}; color: {t.ink_2};",
                            "#{tag}"
                        }
                    }
                }
            }
            if let (Some(kind), Some(url)) = (undrawn_kind, page) {
                div { class: "guest-type-hint",
                    style: "display: flex; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 8px; border: 1px solid {t.rule}; background: {t.pill}; font-size: 13px; color: {t.ink_2};",
                    span { style: "flex: 1;", "A {kind}: its page shows it as its sharer does." }
                    button { style: "border: 0; background: transparent; color: {t.accent}; cursor: pointer; font-size: 13px;",
                        onclick: move |_| open_in_browser(&url),
                        "Open in browser"
                    }
                }
            }
            match drawn {
                Some((claim, source)) => rsx! {
                    // A guest's content: drawn only by the extension's
                    // page, on a host with no action, no host-service call
                    // and no vault of the person's.
                    architect_extensions::embeds::GuestEmbedView {
                        claim,
                        source,
                        props: serde_json::json!({ "guest": true }),
                        fallback: plain,
                    }
                },
                None => plain,
            }
        }
    }
}

/// A page's words as plain blocks: text as text, images from the link.
#[component]
fn PlainBlocks(guest: Guest, blocks: Vec<PlainBlock>, page: Option<String>) -> Element {
    let t = use_shell().theme;
    let images = guest.images.read().clone();
    rsx! {
        for (i, b) in blocks.into_iter().enumerate() {
            {
                let key = i.to_string();
                match b.kind {
                    BlockKind::Heading => {
                        let size = 24_i32.saturating_sub(2_i32.saturating_mul(i32::from(b.level))).max(14);
                        rsx! { div { key: "{key}", style: "font-size: {size}px; font-weight: 700; color: {t.ink}; margin-top: 6px;", "{b.text}" } }
                    }
                    BlockKind::Paragraph => rsx! {
                        div { key: "{key}", style: "font-size: 15px; line-height: 1.6; color: {t.ink_2}; white-space: pre-wrap;", "{b.text}" }
                    },
                    BlockKind::Quote => rsx! {
                        div { key: "{key}", style: "font-size: 15px; line-height: 1.6; color: {t.ink_2}; border-left: 3px solid {t.rule}; padding-left: 12px;", "{b.text}" }
                    },
                    BlockKind::List => {
                        let ordered = b.level == 1;
                        rsx! {
                            div { key: "{key}", style: "display: flex; flex-direction: column; gap: 4px; padding-left: 4px;",
                                for (n, item) in b.items.iter().enumerate() {
                                    div { key: "{n}", style: "font-size: 15px; line-height: 1.5; color: {t.ink_2};",
                                        if ordered { "{n.saturating_add(1)}. {item}" } else { "• {item}" }
                                    }
                                }
                            }
                        }
                    }
                    BlockKind::Code => rsx! {
                        pre { key: "{key}",
                            style: "margin: 0; padding: 10px 12px; border-radius: 8px; background: {t.pill}; color: {t.ink}; font-family: ui-monospace, Menlo, monospace; font-size: 13px; white-space: pre-wrap;",
                            "{b.text}"
                        }
                    },
                    BlockKind::Rule => rsx! { div { key: "{key}", style: "height: 1px; background: {t.rule}; margin: 8px 0;" } },
                    BlockKind::Image => {
                        let src = b.src.clone().unwrap_or_default();
                        match images.get(&src) {
                            Some(data) => rsx! { img { key: "{key}", src: "{data}", alt: "{b.text}", style: "max-width: 100%; border-radius: 8px;" } },
                            None => rsx! { div { key: "{key}", style: "font-size: 13px; color: {t.ink_3};", "Image: {b.text}" } },
                        }
                    }
                    BlockKind::Audio | BlockKind::Video | BlockKind::Drawn => {
                        let what = match b.kind {
                            BlockKind::Audio => "Audio",
                            BlockKind::Video => "Video",
                            _ => "On its page",
                        };
                        let page = page.clone();
                        rsx! {
                            div { key: "{key}", class: "guest-media",
                                style: "display: flex; align-items: center; gap: 10px; padding: 10px 12px; border-radius: 8px; border: 1px solid {t.rule}; font-size: 13px;",
                                span { style: "color: {t.ink_3}; flex: none;", "{what}" }
                                span { style: "flex: 1; min-width: 0; color: {t.ink_2}; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;", "{b.text}" }
                                if let Some(url) = page {
                                    button { style: "border: 0; background: transparent; color: {t.accent}; cursor: pointer; font-size: 13px; flex: none;",
                                        onclick: move |_| open_in_browser(&url),
                                        "Open in browser"
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "merrybrightstack";

    #[test]
    fn a_launch_opens_only_its_share_links() {
        let args = [
            "--headless".to_owned(),
            format!("https://n.example/s/{CODE}/"),
            "architect://vault".to_owned(),
            format!("file:///s/{CODE}/"),
            architect_spaces::guest::app_link(&format!("https://n.example/s/{CODE}/joy/")),
        ];
        let found = links_in(args);
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn the_os_asks_first_for_a_private_network_the_person_does_not() {
        let local = ShareUrl::parse(&format!("http://127.0.0.1:7422/s/{CODE}/")).unwrap();
        let lan = ShareUrl::parse(&format!("https://192.168.1.4/s/{CODE}/")).unwrap();
        let known = ["http://127.0.0.1:7422".to_owned()];
        assert_eq!(asks_first(&local, Source::Os, &[]), Ask::Local);
        assert_eq!(asks_first(&lan, Source::Os, &[]), Ask::Local);
        // A private node the account uses still asks: the prompt stays.
        assert_eq!(asks_first(&local, Source::Os, &known), Ask::Local);
        assert_eq!(asks_first(&local, Source::Agent, &[]), Ask::Local);
        assert_eq!(asks_first(&local, Source::Person, &[]), Ask::No);
    }

    #[test]
    fn an_unknown_node_asks_first_a_known_one_opens() {
        let url = |host: &str| ShareUrl::parse(&format!("https://{host}/s/{CODE}/")).unwrap();
        let known = vec![
            "https://node.example".to_owned(),
            "https://Other.Example:8443/".to_owned(),
        ];
        for from in [Source::Os, Source::Agent] {
            assert_eq!(asks_first(&url(HOME_HOST), from, &[]), Ask::No);
            assert_eq!(
                asks_first(&url("ARCHITECT.fasttrackstudio.app"), from, &[]),
                Ask::No
            );
            assert_eq!(asks_first(&url("node.example"), from, &known), Ask::No);
            assert_eq!(
                asks_first(&url("other.example:8443"), from, &known),
                Ask::No
            );
            assert_eq!(asks_first(&url("evil.example"), from, &known), Ask::Unknown);
            assert_eq!(
                asks_first(&url("node.example.evil.example"), from, &known),
                Ask::Unknown
            );
            assert_eq!(
                asks_first(&url("fasttrackstudio.app"), from, &known),
                Ask::Unknown
            );
            assert_eq!(asks_first(&url("node.example"), from, &[]), Ask::Unknown);
        }
        // The app scheme names the same node as the link inside it.
        let app = ShareUrl::parse(&architect_spaces::guest::app_link(&format!(
            "https://evil.example/s/{CODE}/"
        )))
        .unwrap();
        assert_eq!(asks_first(&app, Source::Os, &known), Ask::Unknown);
        // The person's own link opens, as in a browser's address bar.
        assert_eq!(
            asks_first(&url("evil.example"), Source::Person, &known),
            Ask::No
        );
    }

    #[test]
    fn an_agents_share_open_is_asked_about_the_palettes_is_not() {
        use architect::action::{ActionCaller, Invocation, InvocationSource};
        let palette =
            ActionCaller::app().with_invocation(Invocation::new(InvocationSource::Palette));
        let agent = ActionCaller::app().with_invocation(Invocation::new(InvocationSource::Agent));
        let script = ActionCaller::app().with_invocation(Invocation::new(InvocationSource::Script));
        let peer = ActionCaller::new(architect::action::Principal::Service {
            name: "model".to_owned(),
        });
        let key = ActionCaller::app().with_invocation(Invocation::new(InvocationSource::Key));
        let menu = ActionCaller::app().with_invocation(Invocation::new(InvocationSource::Menu));
        assert_eq!(source_of(&palette), Source::Person);
        assert_eq!(source_of(&key), Source::Person);
        assert_eq!(source_of(&menu), Source::Person);
        // A run that says nothing of where it came from (an extension's
        // page, as `AppHost` runs it with no invocation) is not the person's.
        assert_eq!(source_of(&ActionCaller::app()), Source::Agent);
        assert_eq!(source_of(&agent), Source::Agent);
        assert_eq!(source_of(&script), Source::Agent);
        assert_eq!(source_of(&peer), Source::Agent);
    }

    #[test]
    fn an_extension_pages_share_open_asks_first() {
        use architect::action::{ActionCaller, Invocation, InvocationSource};
        // As `architect_pages::host::AppHost` runs a page's action.
        let page = ActionCaller::app().with_invocation(Invocation::new(InvocationSource::Script));
        let from = source_of(&page);
        assert_eq!(from, Source::Agent);
        let unknown = ShareUrl::parse(&format!("https://evil.example/s/{CODE}/")).unwrap();
        let lan = ShareUrl::parse(&format!("https://10.0.0.5/s/{CODE}/")).unwrap();
        assert_eq!(asks_first(&unknown, from, &[]), Ask::Unknown);
        assert_eq!(asks_first(&lan, from, &[]), Ask::Local);
        // And once it opens, its host is still checked.
        let opened = Opened {
            url: Some(unknown),
            phase: Phase::Loading,
            note: None,
            from,
            local_ok: false,
        };
        assert!(opened.checks_address());
    }

    fn opened(from: Source, phase: Phase, local_ok: bool) -> Opened {
        Opened {
            url: Some(ShareUrl::parse(&format!("https://evil.example/s/{CODE}/")).unwrap()),
            phase,
            note: None,
            from,
            local_ok,
        }
    }

    #[test]
    fn trying_again_checks_the_address_again_unless_the_person_said_local_is_fine() {
        // Yes to an unknown node, then it timed out: "Try again" looks the
        // host up again (the DNS may now answer 10.0.0.5) and pins it.
        let yes_unknown = opened(Source::Os, Phase::Asking(Ask::Unknown), false);
        let local_ok = yes_unknown.local_ok_after_yes();
        assert!(!local_ok);
        let failed = opened(Source::Os, Phase::Failed(Why::Unreachable), local_ok);
        assert!(failed.checks_address());
        assert!(!failed.local_ok_after_yes());
        let agent = opened(Source::Agent, Phase::Failed(Why::Building), false);
        assert!(agent.checks_address());
        // Yes to a private network: kept across "Try again".
        let yes_local = opened(Source::Os, Phase::Asking(Ask::Local), false);
        assert!(yes_local.local_ok_after_yes());
        let failed = opened(Source::Os, Phase::Failed(Why::Unreachable), true);
        assert!(!failed.checks_address());
        assert!(failed.local_ok_after_yes());
        // The person's own link is as a browser's address bar.
        assert!(!opened(Source::Person, Phase::Failed(Why::Unreachable), false).checks_address());
    }

    #[test]
    fn a_host_that_does_not_resolve_is_not_read() {
        let public: std::net::SocketAddr = "93.184.216.34:443".parse().unwrap();
        assert!(after_lookup(Resolved::Unknown) == Err(Phase::Failed(Why::Unreachable)));
        assert!(after_lookup(Resolved::Local) == Err(Phase::Asking(Ask::Local)));
        assert!(after_lookup(Resolved::Public(vec![public])) == Ok(vec![public]));
    }

    #[test]
    fn a_known_node_is_known_on_its_own_port_alone() {
        let url = |host: &str| ShareUrl::parse(&format!("https://{host}/s/{CODE}/")).unwrap();
        let known = vec![
            "https://node.example".to_owned(),
            "https://other.example:8443/".to_owned(),
            "bare.example".to_owned(),
            "pinned.example:9443".to_owned(),
        ];
        assert!(known_host(&url(HOME_HOST), &[]));
        assert!(known_host(&url(&format!("{HOME_HOST}:443")), &[]));
        assert!(!known_host(&url(&format!("{HOME_HOST}:8443")), &[]));
        assert!(known_host(&url("node.example"), &known));
        assert!(!known_host(&url("node.example:8443"), &known));
        assert!(known_host(&url("other.example:8443"), &known));
        assert!(!known_host(&url("other.example"), &known));
        assert!(!known_host(&url("other.example:9000"), &known));
        // A bare name names no port.
        assert!(known_host(&url("bare.example:7000"), &known));
        assert!(known_host(&url("pinned.example:9443"), &known));
        assert!(!known_host(&url("pinned.example"), &known));
        let local = ShareUrl::parse(&format!("http://127.0.0.1:7422/s/{CODE}/")).unwrap();
        assert!(known_host(&local, &["http://127.0.0.1:7422/".to_owned()]));
        assert!(!known_host(&local, &["http://127.0.0.1:7423/".to_owned()]));
    }

    #[test]
    fn a_session_goes_only_to_a_node_the_account_uses() {
        let hosts = vec![
            "https://node.example".to_owned(),
            "http://127.0.0.1:7422/".to_owned(),
        ];
        assert!(trusts(&hosts, "https://node.example"));
        assert!(trusts(&hosts, "http://127.0.0.1:7422"));
        assert!(!trusts(&hosts, "https://evil.example"));
        assert!(!trusts(&hosts, "https://node.example.evil.example"));
        assert!(!trusts(&[], "https://node.example"));
    }

    #[test]
    fn an_image_is_fetched_only_from_below_its_link() {
        let link = ShareUrl::parse(&format!("https://n.example/s/{CODE}/")).unwrap();
        let ok = |src: &str| image_url(&link, src);
        assert_eq!(
            ok(&format!("/s/{CODE}/Art/joy.png")).as_deref(),
            Some(format!("https://n.example/s/{CODE}/Art/joy.png").as_str())
        );
        for bad in [
            format!("/s/{CODE}/%2e%2e/x.png"),
            format!("/s/{CODE}/%2E%2E/x.png"),
            format!("/s/{CODE}/../x.png"),
            format!("/s/{CODE}/a/../../x.png"),
            format!("/s/{CODE}/a%2f..%2fx.png"),
            format!("/s/{CODE}/a%5cx.png"),
            format!("/s/{CODE}//x.png"),
            format!("/s/{CODE}/x.png?y=1"),
            format!("/s/{CODE}/x.png#y"),
            format!("/s/{CODE}/"),
            format!("/s/{CODE}"),
            format!("https://evil.example/s/{CODE}/x.png"),
            format!("//evil.example/s/{CODE}/x.png"),
            format!("https://u:p@n.example/s/{CODE}/x.png"),
            format!("http://n.example/s/{CODE}/x.png"),
            "/s/othercodeothercode/x.png".to_owned(),
            "/x.png".to_owned(),
        ] {
            assert_eq!(ok(&bad), None, "{bad}");
        }
    }

    #[test]
    fn no_words_carry_the_link() {
        for why in [
            Why::Gone,
            Why::SignIn,
            Why::Denied,
            Why::Building,
            Why::Unreachable,
            Why::Broken,
        ] {
            let (title, line) = why.words("n.example");
            assert!(!title.contains(CODE) && !line.contains(CODE));
        }
    }
}
