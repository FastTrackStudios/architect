//! A share link opened as a guest (#629): Architect itself, not only the
//! page, reads a link someone shared, in the app or in a browser.
//!
//! - [`ShareUrl`]: a link, parsed and checked before anything is sent.
//!   Only `https:`, or `http:` to this machine (as a browser's secure
//!   contexts); never `file:` or another scheme, never credentials in it,
//!   and only a share link's path (`/s/<code>/…`, [`crate::share::LINKS`]).
//!   [`ShareUrl::is_local`] says when it points at this machine or a
//!   private network, which an app confirms before opening a link the OS
//!   handed it.
//! - [`ShareGuest`]: the read-only host service a guest of a link calls,
//!   as `Principal::Guest { link_id: <the link's code> }`: the link's
//!   notes in list order, a note, its files at the link's layer. The node
//!   answers it with the same checks as the link's pages (live, audience,
//!   the audience's filter, layers), in one place. Its HTTP face is
//!   `POST /s/<code>/_guest/<method>`, under the link itself.
//!
//! The model and the trait are wasm-clean: a browser calls the service
//! with `ShareGuestHttpClient` (feature `http-client` of `architect`).

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use architect_permissions::{MethodPermit, ServicePermits};
use facet::Facet;

use crate::share::{LINKS, is_code, link_path};

/// The scheme an app registers for links (#186): `architect://open?url=…`
/// opens the share link in `url`.
pub const APP_SCHEME: &str = "architect";

/// What a share link's page and the web app it loads must agree on.
///
/// The page is the node's copy of the web app's page (#646): the node
/// writes this in its `<meta name="architect-share-shell">`, the app compares it
/// with its own and, when they differ, says it is out of date instead of
/// drawing a link it may read wrong. Raise it whenever the page, the
/// guest service or the app's share route change in a way the other side
/// must know.
pub const SHARE_SHELL: u32 = 1;

/// The `<meta name>` that carries [`SHARE_SHELL`] on a share link's page.
pub const SHARE_SHELL_META: &str = "architect-share-shell";

/// The `<meta name>` that carries the node's own version on a share
/// link's page (`<version>` or `<version>+<rev>`), for what the app says
/// when it is out of date.
pub const NODE_VERSION_META: &str = "architect-node-version";

/// The id of the line a share link's page shows when the app has not
/// drawn the link a while after it loaded.
///
/// That is a web app too old to know share links, or none served at all.
/// The app removes it once it is showing the link.
pub const SHARE_STALL_ID: &str = "share-stall";

/// Why a link is not opened. Its words never carry the link (its code
/// is a credential).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Not a URL.
    NotAUrl,
    /// A scheme other than `https:` (or `http:` to this machine).
    Scheme(String),
    /// `http:` to anywhere but this machine.
    Insecure,
    /// A user name or password in it.
    Credentials,
    /// Not a share link's path (`/s/<code>/…`), or a sign-in's.
    NotAShareLink,
    /// No host, or one that does not read.
    Host,
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAUrl => f.write_str("that is not a link"),
            Self::Scheme(s) => write!(f, "links starting `{s}:` are not opened"),
            Self::Insecure => {
                f.write_str("only https links are opened (http only to this computer)")
            }
            Self::Credentials => f.write_str("a link with a name or password in it is not opened"),
            Self::NotAShareLink => f.write_str("that is not a share link"),
            Self::Host => f.write_str("that link names no host"),
        }
    }
}

impl std::error::Error for Refused {}

/// A share link: its node (origin), its code, and the path below it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareUrl {
    https: bool,
    /// As the URL parser wrote it: lower case, punycode, `[v6]`.
    host: String,
    port: Option<u16>,
    code: String,
    /// Below the link, from `/` (`/`, `/joy/`, `/Set/joy.pdf`).
    rest: String,
}

impl ShareUrl {
    /// Read a share link, or an app link that carries one
    /// (`architect://open?url=<the link>`).
    ///
    /// # Errors
    ///
    /// [`Refused`]: not a URL, a scheme other than `https:` (`http:` only
    /// to this machine), credentials in it, no host, or not a share
    /// link's path. The node's sign-in (`/s/_…`) is not a share link.
    pub fn parse(input: &str) -> Result<Self, Refused> {
        let input = input.trim();
        let url = url::Url::parse(input).map_err(|_| Refused::NotAUrl)?;
        if url.scheme() == APP_SCHEME {
            if url.host_str() != Some("open") {
                return Err(Refused::NotAShareLink);
            }
            let inner = url
                .query_pairs()
                .find(|(k, _)| k == "url")
                .map(|(_, v)| v.into_owned())
                .ok_or(Refused::NotAShareLink)?;
            // One level: an app link inside an app link is not followed.
            let inner_url = url::Url::parse(inner.trim()).map_err(|_| Refused::NotAUrl)?;
            if inner_url.scheme() == APP_SCHEME {
                return Err(Refused::Scheme(APP_SCHEME.to_owned()));
            }
            return Self::from_url(&inner_url);
        }
        Self::from_url(&url)
    }

    fn from_url(url: &url::Url) -> Result<Self, Refused> {
        let https = match url.scheme() {
            "https" => true,
            "http" => false,
            other => return Err(Refused::Scheme(other.to_owned())),
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Refused::Credentials);
        }
        let host = match url.host() {
            Some(url::Host::Domain(d)) if !d.is_empty() => d.to_owned(),
            Some(url::Host::Ipv4(ip)) => ip.to_string(),
            Some(url::Host::Ipv6(ip)) => format!("[{ip}]"),
            _ => return Err(Refused::Host),
        };
        let this = Self {
            https,
            host,
            port: url.port(),
            code: String::new(),
            rest: String::new(),
        };
        if !https && !this.is_loopback() {
            return Err(Refused::Insecure);
        }
        let below = url
            .path()
            .strip_prefix(LINKS)
            .and_then(|r| r.strip_prefix('/'))
            .ok_or(Refused::NotAShareLink)?;
        let (code, rest) = below.split_once('/').unwrap_or((below, ""));
        if !is_code(code) {
            return Err(Refused::NotAShareLink);
        }
        Ok(Self {
            code: code.to_owned(),
            rest: format!("/{rest}"),
            ..this
        })
    }

    /// The node: `https://host[:port]`, where every request goes and
    /// nowhere else.
    #[must_use]
    pub fn origin(&self) -> String {
        let scheme = if self.https { "https" } else { "http" };
        format!("{scheme}://{}", self.host_label())
    }

    /// What to call the node to a person: its host (and port): never
    /// the link.
    #[must_use]
    pub fn host_label(&self) -> String {
        self.port
            .map_or_else(|| self.host.clone(), |port| format!("{}:{port}", self.host))
    }

    /// The host alone, as written in the URL.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// The link's code: a credential (never logged, never shown).
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Below the link, from `/`: which page or file it opened at.
    #[must_use]
    pub fn rest(&self) -> &str {
        &self.rest
    }

    /// The link's own path: `/s/<code>/`.
    #[must_use]
    pub fn mount(&self) -> String {
        link_path(&self.code)
    }

    /// The link, at its root.
    #[must_use]
    pub fn link(&self) -> String {
        format!("{}{}", self.origin(), self.mount())
    }

    /// A path below the link (`/joy/`) as a URL on its node, for "Open
    /// in browser"; the link's root for anything that is not one.
    #[must_use]
    pub fn page_url(&self, below: &str) -> String {
        let below = below.trim_start_matches('/');
        let safe = !below.split('/').any(|s| s == ".." || s == ".");
        if safe {
            format!("{}{}", self.link(), below)
        } else {
            self.link()
        }
    }

    /// This link as an app link: `architect://open?url=<the link>`.
    #[must_use]
    pub fn app_link(&self) -> String {
        app_link(&format!("{}{}", self.origin(), self.mount_and_rest()))
    }

    fn mount_and_rest(&self) -> String {
        format!(
            "{}{}",
            self.mount().trim_end_matches('/'),
            if self.rest.is_empty() {
                "/"
            } else {
                &self.rest
            }
        )
    }

    /// Whether the host is this machine (`localhost`, a loopback
    /// address).
    #[must_use]
    pub fn is_loopback(&self) -> bool {
        let host = self.host.trim_start_matches('[').trim_end_matches(']');
        host == "localhost"
            || host.ends_with(".localhost")
            || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
    }

    /// Whether the host is this machine or a private network: loopback,
    /// a private, link-local, shared (CGNAT) or unique-local address, or
    /// a `.local`, `.internal`, `.lan` or `.home.arpa` name. A link the
    /// OS handed the app is opened only after the person says so (as a
    /// name may resolve to one, a native client checks the address it
    /// connects to too: [`is_private_address`]).
    #[must_use]
    pub fn is_local(&self) -> bool {
        let host = self.host.trim_start_matches('[').trim_end_matches(']');
        if self.is_loopback() {
            return true;
        }
        if let Ok(ip) = host.parse::<IpAddr>() {
            return is_private_address(ip);
        }
        let host = host.trim_end_matches('.');
        !host.contains('.')
            || [".local", ".internal", ".lan", ".home.arpa", ".intranet"]
                .iter()
                .any(|s| host.ends_with(s))
    }
}

/// `architect://open?url=<url>`, `url` percent-encoded.
#[must_use]
pub fn app_link(url: &str) -> String {
    let mut out = format!("{APP_SCHEME}://open?url=");
    for b in url.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// Whether `ip` is this machine or a network that is not the internet.
///
/// Loopback, unspecified, private (`10/8`, `172.16/12`, `192.168/16`),
/// shared (`100.64/10`), link-local, multicast, broadcast, documentation,
/// or for IPv6 unique-local (`fc00::/7`), link-local (`fe80::/10`) and an
/// IPv4 one mapped into IPv6.
#[must_use]
pub fn is_private_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => private_v4(v4),
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return private_v4(v4);
            }
            private_v6(v6)
        }
    }
}

fn private_v4(ip: Ipv4Addr) -> bool {
    let [a, b, ..] = ip.octets();
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_private()
        || ip.is_link_local()
        || ip.is_multicast()
        || ip.is_broadcast()
        || ip.is_documentation()
        || (a == 100 && (64..128).contains(&b))
        || a == 0
}

fn private_v6(ip: Ipv6Addr) -> bool {
    let first = ip.segments().first().copied().unwrap_or(0);
    ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        || (first & 0xfe00) == 0xfc00
        || (first & 0xffc0) == 0xfe80
}

// ── The service ───────────────────────────────────────────────────────

/// A link as its guest sees it: what it is called, how far it plays, and
/// its notes in list order.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct GuestLink {
    /// The link's path on its node (`/s/<code>/`).
    pub link: String,
    pub title: String,
    /// How far the link plays: `page`, `reference` or `full`.
    pub listen: String,
    /// Who the link is for: `public`, `link`, `org`, `invited`.
    pub audience: String,
    /// Its notes, in list order.
    pub notes: Vec<ListedNote>,
}

/// One note a link lists.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ListedNote {
    /// Its path in the vault: what [`ShareGuest::read`] takes.
    pub path: String,
    pub title: String,
    /// Its `type` property, when it has one: what an extension may claim.
    pub kind: Option<String>,
    /// Its page below the node (`/s/<code>/joy/`), when it has one.
    pub page: Option<String>,
    /// The files it attaches that the link serves.
    pub files: Vec<String>,
    /// Its art (`cover:`/`art:`), one of `files`: a list's tile and the
    /// Now Playing card's picture (#646). None when it has none, or the
    /// link does not serve it.
    #[facet(default)]
    pub art: Option<String>,
    /// Its subtitle as a media item (`subtitle:`, else `artist:`; #668).
    #[facet(default)]
    pub subtitle: Option<String>,
    /// Its length as a media item (`duration:`), in milliseconds.
    #[facet(default)]
    pub duration_ms: Option<u64>,
    /// What it plays as a media item (`media:`, else `recording:`): one of
    /// `files`. None when it names none the link serves.
    #[facet(default)]
    pub media: Option<String>,
    /// Its list when it is a collection (#668: `sets`/`songs` or
    /// `sections`/`items`), in order: each section's name and the notes it
    /// names, by vault path, only notes this link lists (a link to any
    /// other is left out). Empty when it is none.
    #[facet(default)]
    pub sets: Vec<ListedSet>,
}

/// One section of a listed collection: its name, and its notes' paths,
/// each a note the link lists.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ListedSet {
    pub name: Option<String>,
    pub notes: Vec<String>,
}

/// One file a link serves, at the layer asked for.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ListedFile {
    /// Its path in the vault.
    pub path: String,
    /// Where its bytes are (ranged): a path on the node.
    pub url: String,
    pub size: u64,
    /// None until the vault has hashed it.
    pub sha256: Option<String>,
    /// Its layer: `page`, `reference` or `full`; None while where files
    /// are placed is not known.
    pub layer: Option<String>,
    /// The note that attaches it.
    pub note: String,
}

/// A note as its guest reads it.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct GuestNote {
    pub path: String,
    pub title: String,
    /// Its `type` property: what an extension may claim to draw it.
    pub kind: Option<String>,
    /// Its page below the node (`/s/<code>/joy/`).
    pub page: String,
    pub tags: Vec<String>,
    /// What its page says, as plain blocks: the page's own prose, made
    /// safe as the page's is, never its markdown.
    pub blocks: Vec<PlainBlock>,
    /// Its text after the frontmatter, for the extension whose view draws
    /// its type (#648, #669): given only when the node's island that
    /// claims the type reads the note (`reads = ["documents"]`), and then
    /// as the share's build wrote it: `%%comments%%` and HTML gone
    /// ([`crate::view::without_comments`]), a link to anything outside
    /// the link reduced to its words, an embed of it gone. Its
    /// frontmatter never. None otherwise: the note is drawn from
    /// `blocks`.
    pub source: Option<String>,
    /// The link's files the view that draws its type may read (#669): only
    /// when that island declares `reads = ["files"]` too; images and
    /// synced lyrics the link serves at its layer, the note's own first,
    /// within [`crate::view::within_caps`]; never one the vault has not
    /// hashed yet. Each is fetched from its `url` (below the link, checked
    /// as every request for it is) and checked against its `sha256`; the
    /// app takes none without one.
    #[facet(default)]
    pub files: Vec<ListedFile>,
}

/// What a block of a page is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum BlockKind {
    /// `level` 1–6, `text`.
    Heading,
    /// `text`.
    Paragraph,
    /// `items`, `ordered` by `level == 1`.
    List,
    /// `text`.
    Quote,
    /// `text`, `info` the fence's language.
    Code,
    /// `src` (below the link), `text` its alt.
    Image,
    /// `src`, `text` its name.
    Audio,
    /// `src`, `text` its name.
    Video,
    /// A rule.
    Rule,
    /// A block an extension drew on the page (a chart, a base): drawn
    /// there, `text` what it says, `info` its claim when known.
    Drawn,
}

/// One block of a page, plain: text and the URLs of its media, nothing
/// that runs or styles.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct PlainBlock {
    pub kind: BlockKind,
    pub level: u8,
    pub text: String,
    pub items: Vec<String>,
    /// A medium's URL: a path on the link's node, below the link.
    pub src: Option<String>,
    pub info: Option<String>,
}

impl PlainBlock {
    /// A block of `kind` saying `text`.
    #[must_use]
    pub fn new(kind: BlockKind, text: impl Into<String>) -> Self {
        Self {
            kind,
            level: 0,
            text: text.into(),
            items: Vec::new(),
            src: None,
            info: None,
        }
    }
}

/// Why a guest's call was refused. Its words never carry the code.
#[architect::error]
#[derive(Eq)]
pub enum GuestFault {
    /// Expired, revoked, never there, or a note or file this link does
    /// not serve: the same answer, so none tells the others apart.
    #[error("there is nothing at this link")]
    #[architect(http_status = 404)]
    Gone,
    /// The link is for signed-in people.
    #[error("this link is for signed-in people")]
    #[architect(http_status = 401)]
    SignIn,
    /// Signed in, but not as someone it was shared with.
    #[error("this link was not shared with this account")]
    #[architect(http_status = 403)]
    Denied,
    /// A file above how far the link plays: the layer it needs.
    #[error("this file needs the {0} layer")]
    #[architect(http_status = 403)]
    Held(String),
    /// Its pages are being built, or where its files are is not known.
    #[error("this link is being prepared; try again in a moment")]
    #[architect(http_status = 503)]
    Building,
}

/// The read-only host service of a share link's guest (#629).
///
/// Every call is made as `Principal::Guest { link_id: code }` and checked
/// by the node exactly as a request for the link's page or file is: the
/// link is live, its audience admits the caller (`token`: an issuer
/// session, for a link for signed-in people; empty otherwise), the
/// audience's filter holds for the note now, and a file is within the
/// link's layer. What a guest reads is what the page gives the same
/// visitor, as data.
#[architect::service(path = "s/{code}/_guest")]
pub trait ShareGuest {
    /// The link: its title, layer and audience, and its notes in list
    /// order.
    async fn link(&self, code: String, token: String) -> Result<GuestLink, GuestFault>;

    /// One note the link lists, by its vault path: what its page says.
    async fn read(
        &self,
        code: String,
        token: String,
        path: String,
    ) -> Result<GuestNote, GuestFault>;

    /// The files the link serves, in list order, at `listen` (never past
    /// the link's; None: the link's).
    async fn files(
        &self,
        code: String,
        token: String,
        listen: Option<String>,
    ) -> Result<Vec<ListedFile>, GuestFault>;
}

/// The gate's table for [`ShareGuest`]: reads of the link's own scope.
/// A guest holds nothing else.
pub const GUEST_PERMITS: ServicePermits = ServicePermits {
    service: "ShareGuest",
    methods: &[
        MethodPermit::new("link", "read", "shares/{code}"),
        MethodPermit::new("read", "read", "shares/{code}"),
        MethodPermit::new("files", "read", "shares/{code}"),
    ],
};

#[cfg(test)]
mod tests {
    use super::*;

    const CODE: &str = "merrybrightstack";

    #[test]
    fn a_share_link_reads_with_its_node_code_and_page() {
        let u = ShareUrl::parse(&format!("https://Node.Example.com/s/{CODE}/joy/")).unwrap();
        assert_eq!(u.origin(), "https://node.example.com");
        assert_eq!(u.code(), CODE);
        assert_eq!(u.rest(), "/joy/");
        assert_eq!(u.mount(), format!("/s/{CODE}/"));
        assert!(!u.is_local());
        let root = ShareUrl::parse(&format!("https://n.example/s/{CODE}")).unwrap();
        assert_eq!(root.rest(), "/");
        let port = ShareUrl::parse(&format!("http://127.0.0.1:7422/s/{CODE}/")).unwrap();
        assert_eq!(port.origin(), "http://127.0.0.1:7422");
        assert_eq!(port.host_label(), "127.0.0.1:7422");
        assert!(port.is_loopback() && port.is_local());
    }

    #[test]
    fn only_https_or_http_to_this_machine() {
        let refused = |s: &str| ShareUrl::parse(s).unwrap_err();
        assert_eq!(
            refused(&format!("file:///s/{CODE}/")),
            Refused::Scheme("file".into())
        );
        assert_eq!(
            refused("javascript:alert(1)"),
            Refused::Scheme("javascript".into())
        );
        assert_eq!(
            refused(&format!("http://example.com/s/{CODE}/")),
            Refused::Insecure
        );
        assert_eq!(
            refused(&format!("http://10.0.0.2/s/{CODE}/")),
            Refused::Insecure
        );
        assert_eq!(
            refused(&format!("https://u:p@example.com/s/{CODE}/")),
            Refused::Credentials
        );
        assert_eq!(refused("https://example.com/docs/"), Refused::NotAShareLink);
        assert_eq!(
            refused("https://example.com/s/_signin?to=/"),
            Refused::NotAShareLink
        );
        assert_eq!(
            refused("https://example.com/s/short/"),
            Refused::NotAShareLink
        );
        assert_eq!(refused("not a link"), Refused::NotAUrl);
        assert!(ShareUrl::parse(&format!("http://localhost:8080/s/{CODE}/")).is_ok());
        assert!(ShareUrl::parse(&format!("http://[::1]:8080/s/{CODE}/")).is_ok());
        // A refusal's words never carry the link.
        for r in [
            Refused::Insecure,
            Refused::NotAShareLink,
            Refused::Scheme("x".into()),
        ] {
            assert!(!r.to_string().contains(CODE));
        }
    }

    #[test]
    fn an_app_link_carries_one_share_link() {
        let link = format!("https://node.example/s/{CODE}/joy/");
        let app = app_link(&link);
        assert!(app.starts_with("architect://open?url=https%3A%2F%2F"));
        let back = ShareUrl::parse(&app).unwrap();
        assert_eq!(back, ShareUrl::parse(&link).unwrap());
        assert_eq!(back.app_link(), app);
        // Nested, another verb, or a refused link inside: refused.
        assert!(ShareUrl::parse(&app_link(&app)).is_err());
        assert!(ShareUrl::parse("architect://vault").is_err());
        assert_eq!(
            ShareUrl::parse(&app_link("file:///etc/passwd")).unwrap_err(),
            Refused::Scheme("file".into())
        );
    }

    #[test]
    fn private_networks_are_local() {
        let local = |h: &str| {
            ShareUrl::parse(&format!("https://{h}/s/{CODE}/"))
                .unwrap()
                .is_local()
        };
        for h in [
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.10",
            "100.64.0.1",
            "169.254.1.1",
            "[fd00::1]",
            "[fe80::1]",
            "[::ffff:192.168.0.1]",
            "nas.local",
            "printer",
            "box.internal",
            "0.0.0.0",
        ] {
            assert!(local(h), "{h} is local");
        }
        for h in [
            "architect.fasttrackstudio.app",
            "8.8.8.8",
            "[2606:4700::1111]",
        ] {
            assert!(!local(h), "{h} is not local");
        }
    }

    #[test]
    fn page_urls_stay_below_the_link() {
        let u = ShareUrl::parse(&format!("https://n.example/s/{CODE}/")).unwrap();
        assert_eq!(
            u.page_url("/joy/"),
            format!("https://n.example/s/{CODE}/joy/")
        );
        assert_eq!(u.page_url("/../x/"), format!("https://n.example/s/{CODE}/"));
    }

    #[test]
    fn a_guest_note_round_trips_as_json() {
        let note = GuestNote {
            path: "Set/joy.md".into(),
            title: "Joy".into(),
            kind: Some("song".into()),
            page: format!("/s/{CODE}/joy/"),
            tags: vec!["set".into()],
            blocks: vec![PlainBlock::new(BlockKind::Heading, "Joy")],
            source: None,
            files: Vec::new(),
        };
        let json = facet_json::to_string(&note).unwrap();
        let back: GuestNote = facet_json::from_str(&json).unwrap();
        assert_eq!(back, note);
    }
}
