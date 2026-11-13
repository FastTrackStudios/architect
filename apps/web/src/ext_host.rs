//! The extension frame host's decisions (#495, route A), with no browser.
//!
//! What the node's islands listing says and whether to believe it, each
//! file against its digest, the asks a frame may make and how often, the
//! heartbeat, and the theme handed in. The DOM half (the frames, their ports, the timers) is `app::frames`.
//!
//! The frame side is the node's web view document
//! (`features/sites/assets/webview.js`, its policy
//! `architect_sites::island::webview_headers`); this host speaks its
//! protocol, read from the share page's `webPane` and written here in Rust
//! so the app depends on nothing of the share page's script:
//!
//! 1. the app creates `<iframe sandbox="allow-scripts">` on the node's web
//!    view document (an opaque origin), and on its first `load` hands it
//!    one `MessagePort` (`"architect-web"`);
//! 2. the frame says `hello`; the app answers `theme` and `boot` (the
//!    bundle's files as bytes, each fetched without credentials and
//!    checked against the listing's SHA-256 first), once;
//! 3. the frame says `ready`; the app sends `props` (data only) and a
//!    `ping` every [`BEAT_MS`], which the frame answers with `pong`;
//! 4. the frame may say `size` (its height), `ask` (one of
//!    [`INTENTS`], rate-limited by [`Bucket`], answered by the playhead,
//!    `architect_playhead::ask`, as the island's extension) or `error`.
//!
//! An island lists its views ([`Island::views`]: the share page's
//! progress, transport and lyrics slots and its claims) and may claim note
//! types ([`Embed`]): its view is drawn at the head of a note of that
//! `type` ([`claimed`]: the first island listed that claims it), given
//! what [`note_props`] lets through of the note for who is viewing, and
//! may set its source in the app's playhead (`PLAYHEAD_SET`) and make one
//! of its items current (`PLAYHEAD_ITEM`), as `architect.playhead` v0.1
//! has them (#589).
//!
//! The frame dies (taken out, the app's own control back) on a second
//! `load` (it left its document), on `error`, when not ready within
//! [`READY_MS`] or when a ping goes unanswered past [`LATE_MS`]. Nothing
//! the frame is given is a credential: no token, no cookie (its origin is
//! opaque), only the bundle, the theme and the props.

use std::collections::BTreeMap;

use serde_json::Value;

/// Where the node lists its islands (`apps/desktop/src/node/shares.rs`
/// `ISLANDS_LISTING`), on this origin.
pub const LISTING: &str = "/_ext/islands.json";

/// How long a frame has to say it is ready, ms.
pub const READY_MS: u32 = 15_000;
/// How often the app pings a ready frame, ms.
pub const BEAT_MS: u32 = 2_000;
/// How long a ping may go unanswered, ms.
pub const LATE_MS: f64 = 6_000.0;
/// The tallest a frame may say it is, px.
pub const MAX_HEIGHT: f64 = 4_000.0;
/// The most of the window's height a view at a note's head takes.
///
/// At least [`HEAD_MIN`] px; taller, it scrolls inside its own box, so a
/// long list (a 500-item queue) is all there without pushing the note
/// away at 390 px wide (#657, #660).
pub const HEAD_SHARE: f64 = 0.6;
/// The least a view at a note's head is given before it scrolls, px.
pub const HEAD_MIN: f64 = 240.0;

/// How tall a view at a note's head is drawn, px: the height it says
/// (`size`), up to [`HEAD_SHARE`] of a window `viewport` px tall (at
/// least [`HEAD_MIN`]); past that its frame scrolls.
#[must_use]
pub fn head_height(said: f64, viewport: f64) -> f64 {
    let cap = (viewport * HEAD_SHARE).max(HEAD_MIN);
    said.clamp(0.0, cap)
}

/// At most this many asks a second from one frame, with a burst of twice
/// that.
pub const ASKS_PER_SECOND: f64 = 8.0;
/// At most this many asks a second from every frame together, with a
/// burst of twice that: one frame cannot use it up (#657, #660 L2).
pub const ALL_ASKS_PER_SECOND: f64 = 32.0;
/// The most bytes a message from a frame may take as JSON; larger is
/// dropped unread. A `PLAYHEAD_SET` of a queue with its timed marks fits.
pub const MAX_MESSAGE: usize = 64 * 1024;

/// What a frame may ask the app: the playhead's controls of its own
/// extension's source (#589), nothing else (host calls into a vault are
/// #495's next step, authorised in the app and the node). Its props are
/// its source's state, so it reads nothing.
pub use architect_playhead::FRAME_ASKS as INTENTS;

/// The read capability a claim names, and its extension declares
/// (`[capabilities] reads`), for its view to be handed the note it is
/// drawn on: the app's documents.
pub const READS_NOTE: &str = "documents";

/// The read capability that hands a note-reading view the note's files.
///
/// An extension declares it beside [`READS_NOTE`] (#669): art, chart
/// images and synced lyrics, within `architect_spaces::view`'s caps.
pub const READS_FILES: &str = "files";

/// A view of an island drawn at the head of a note whose `type` it claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Embed {
    /// Its view (`mount(view)`).
    pub view: String,
    /// The note types it claims, as listed.
    pub note_types: Vec<String>,
    /// Whether the claim names the note as what it reads (its listing's
    /// `reads` holds [`READS_NOTE`]).
    pub reads_note: bool,
    /// The note's properties it declares for those types: all of them a
    /// share's guest's view is given.
    pub properties: Vec<String>,
}

impl Island {
    /// Every view its listing names, once each, in the listing's order: the
    /// views a favourite's `{view}` of its extension may open.
    #[must_use]
    pub fn views(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let named = [&self.progress, &self.transport, &self.lyrics];
        for v in named
            .into_iter()
            .flatten()
            .chain(self.embeds.iter().map(|e| &e.view))
        {
            if !out.contains(v) {
                out.push(v.clone());
            }
        }
        out
    }

    /// Its claim on notes of type `note_type` (without regard to case):
    /// the first of its embeds listed that claims it.
    #[must_use]
    pub fn claim_for(&self, note_type: &str) -> Option<&Embed> {
        let t = note_type.trim();
        if t.is_empty() {
            return None;
        }
        self.embeds.iter().find(|e| {
            e.note_types
                .iter()
                .any(|n| n.trim().eq_ignore_ascii_case(t))
        })
    }

    /// The view it draws at the head of a note of type `note_type`, if it
    /// claims it.
    #[must_use]
    pub fn embed_for(&self, note_type: &str) -> Option<&str> {
        self.claim_for(note_type).map(|e| e.view.as_str())
    }

    /// Whether its extension declares reading notes ([`READS_NOTE`]).
    #[must_use]
    pub fn declares_reading_notes(&self) -> bool {
        self.reads.iter().any(|r| r == READS_NOTE)
    }

    /// Whether its view on a note of type `note_type` is handed the note's
    /// files ([`note_props`]): its claim reads the note, and its extension
    /// declares [`READS_NOTE`] and [`READS_FILES`]. A caller fetches files
    /// for a view only when this holds.
    #[must_use]
    pub fn reads_files_of(&self, note_type: &str) -> bool {
        self.claim_for(note_type).is_some_and(|c| c.reads_note)
            && self.declares_reading_notes()
            && self.reads.iter().any(|r| r == READS_FILES)
    }
}

/// The island that draws a note of type `note_type`, and its claim.
///
/// The first island in the listing's order that claims the type, and
/// within it the first of its embeds that does. Chosen from the listing alone,
/// before any bundle is fetched; `None` when no island claims it.
#[must_use]
pub fn claimed<'a>(islands: &'a [Island], note_type: &str) -> Option<(&'a Island, &'a Embed)> {
    islands
        .iter()
        .find_map(|i| i.claim_for(note_type).map(|e| (i, e)))
}

/// Who is viewing the note a view is handed: what [`note_props`] lets
/// through depends on it, and every caller has to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience {
    /// A signed-in member of the vault's organization, or the demo
    /// (`/app/?demo`, the person's own vault in memory): the note, its
    /// `%%comments%%` removed.
    Member,
    /// A share link's guest (#629): what the sharer shared (#648, #669).
    /// The note as the link's node handed it (`ShareGuest::read`, never a
    /// vault file): its title, its type, the properties the claim declares
    /// for that type, its text (as the share's build wrote it, the node
    /// giving it only to a view that reads the note) and the link's files
    /// the node gave for it. No vault path.
    Guest,
}

/// A file a view on a note may read, fetched and checked by the app
/// (#669): an image as a `data:` URL, synced lyrics as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewFile {
    /// Its path: in the vault for a member, in the link for a guest.
    pub path: String,
    /// What it is.
    pub kind: architect_spaces::view::ViewFileKind,
    /// Its size, bytes.
    pub size: u64,
    /// An image's `data:` URL, or the lyrics' text.
    pub content: String,
}

impl ViewFile {
    /// `bytes` of the file at `path` as a view is handed it, or why not:
    /// a kind a view reads, a vault path ([`vault_path`]), within
    /// [`architect_spaces::view::VIEW_FILE_MAX`], and lyrics in UTF-8.
    ///
    /// # Errors
    ///
    /// The file is not one a view may be handed.
    pub fn of(path: &str, bytes: &[u8]) -> Result<Self, String> {
        use architect_spaces::view::{VIEW_FILE_MAX, ViewFileKind};
        use base64::Engine as _;
        let kind =
            ViewFileKind::of(path).ok_or_else(|| format!("{path}: not a file a view reads"))?;
        if !vault_path(path) {
            return Err(format!("{path}: not a path in a vault"));
        }
        let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if size > VIEW_FILE_MAX {
            return Err(format!("{path}: larger than {VIEW_FILE_MAX} bytes"));
        }
        let content = match kind {
            ViewFileKind::Image(mime) => format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ),
            ViewFileKind::Lyrics => String::from_utf8(bytes.to_vec())
                .map_err(|_| format!("{path}: lyrics not in UTF-8"))?,
        };
        Ok(Self {
            path: path.to_owned(),
            kind,
            size,
            content,
        })
    }

    /// As a view is handed it: `{path, kind, size, data}` for an image,
    /// `{path, kind, size, text}` for lyrics.
    fn props(&self) -> Value {
        let key = match self.kind {
            architect_spaces::view::ViewFileKind::Image(_) => "data",
            architect_spaces::view::ViewFileKind::Lyrics => "text",
        };
        let mut v = serde_json::json!({
            "path": self.path, "kind": self.kind.word(), "size": self.size,
        });
        if let Some(o) = v.as_object_mut() {
            o.insert(key.to_owned(), Value::String(self.content.clone()));
        }
        v
    }
}

/// A note as the app read it from the vault, before the gate.
#[derive(Debug, Clone, Copy)]
pub struct Note<'a> {
    /// Its vault-relative path.
    pub path: &'a str,
    /// Its title.
    pub title: &'a str,
    /// Its frontmatter `type`.
    pub note_type: &'a str,
    /// Its frontmatter, each value as text.
    pub properties: &'a [(String, String)],
    /// Its markdown after the frontmatter: a member's from the vault, a
    /// guest's as the link's node handed it (`GuestNote::source`; empty
    /// when it gave none).
    pub body: &'a str,
    /// Its files a view may read (#669), fetched and checked by the caller
    /// only when [`Island::reads_files_of`] holds: a member's from their
    /// vault, a guest's only those the link's node handed for the note.
    pub files: &'a [ViewFile],
}

/// What `island`'s views are handed of `note` for `audience`.
///
/// The one gate between a vault's note and a frame, wherever a view is
/// mounted on a note. `null` unless the island claims the note's type with a claim
/// that names [`READS_NOTE`] and its extension declares it too.
///
/// - [`Audience::Member`]: `{path, title, type, properties, body}`, the
///   body without its `%%comments%%`;
/// - [`Audience::Guest`]: `{title, type, properties, body}`, only the
///   properties the claim declares, the body as the node gave it, again
///   without its `%%comments%%`; never a vault path.
///
/// Either has `files` (`[{path, kind, size, data | text}]`) too when its
/// extension declares [`READS_FILES`] ([`Island::reads_files_of`]): the
/// files given, each a kind a view reads at a vault path, within
/// `architect_spaces::view::within_caps`.
#[must_use]
pub fn note_props(audience: Audience, island: &Island, note: &Note<'_>) -> Value {
    let Some(claim) = island.claim_for(note.note_type) else {
        return Value::Null;
    };
    if !claim.reads_note || !island.declares_reading_notes() {
        return Value::Null;
    }
    let mut properties = serde_json::Map::new();
    for (k, v) in note.properties {
        if audience == Audience::Member || claim.properties.iter().any(|p| p == k) {
            properties.insert(k.clone(), Value::String(v.clone()));
        }
    }
    let mut props = match audience {
        Audience::Member => serde_json::json!({
            "path": note.path,
            "title": note.title,
            "type": note.note_type,
            "properties": properties,
            "body": without_comments(note.body),
        }),
        Audience::Guest => serde_json::json!({
            "title": note.title,
            "type": note.note_type,
            "properties": properties,
            "body": without_comments(note.body),
        }),
    };
    if island.reads_files_of(note.note_type)
        && let Some(o) = props.as_object_mut()
    {
        let files = architect_spaces::view::within_caps(
            note.files.iter().filter(|f| {
                vault_path(&f.path)
                    && architect_spaces::view::ViewFileKind::of(&f.path) == Some(f.kind)
            }),
            |f| f.path.as_str(),
            |f| f.size,
        );
        o.insert(
            "files".to_owned(),
            Value::Array(files.into_iter().map(ViewFile::props).collect()),
        );
    }
    props
}

/// `body` without its `%%comments%%`: the one rule, shared with the node
/// that hands a share's guest the same text (`architect_spaces::view`).
pub use architect_spaces::view::without_comments;

/// One island's web bundle, as the listing names it and this host
/// believes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Island {
    /// Its extension's id (`studio.transport`).
    pub id: String,
    /// Its folder on the node, `/_ext/island/<id>.<hash>/`.
    pub folder: String,
    /// The web view document its frames load (`/_ext/webview.<hash>.html`;
    /// never `webview-rtc`, see [`webview_ok`]).
    pub webview: String,
    /// Its wasm-bindgen glue, which exports `mount(view)`.
    pub entry: String,
    /// Every file of the bundle and its `sha256-…` digest.
    pub files: BTreeMap<String, String>,
    /// Its stylesheet and its digest.
    pub sheet: Option<(String, String)>,
    /// Its views: the progress bar, the transport, the lyrics.
    pub progress: Option<String>,
    /// The transport's view.
    pub transport: Option<String>,
    /// The lyrics' view.
    pub lyrics: Option<String>,
    /// Its views drawn at the head of a note by its type.
    pub embeds: Vec<Embed>,
    /// What its extension declares it reads (its manifest's
    /// `[capabilities] reads`), as listed.
    pub reads: Vec<String>,
}

/// Whether `name`'s extension is exactly `ext` (as the node checks it,
/// case and all).
fn ext_is(name: &str, ext: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, e)| e == ext)
}

fn flat_web(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("web-") else {
        return false;
    };
    let ok_ext = ext_is(rest, "js") || ext_is(rest, "wasm");
    ok_ext
        && !rest.is_empty()
        && name.len() <= 128
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// `sha256-` and 44 characters of base64 (32 bytes).
fn pinned(digest: &str) -> bool {
    digest.strip_prefix("sha256-").is_some_and(|b| {
        b.len() == 44
            && b.strip_suffix('=').is_some_and(|head| {
                head.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
            })
    })
}

fn hex_after<'a>(s: &'a str, prefix: &str, suffix: &str) -> Option<&'a str> {
    let mid = s.strip_prefix(prefix)?.strip_suffix(suffix)?;
    (!mid.is_empty() && mid.chars().all(|c| c.is_ascii_hexdigit())).then_some(mid)
}

/// Whether `path` is the node's web view document that blocks WebRTC.
///
/// The `webview-rtc` document is refused, and with it the island that
/// names it: the app grants no extension WebRTC yet (#614), and a listing
/// must not grant it to itself.
///
/// The frame's policy is the node's response headers, which the app
/// cannot read. So the digests guard a bundle's files against a cache or
/// a mirror that swapped one after the listing was made; they are no
/// guard against the node itself, which is the app's own origin and
/// trusted as the app is.
#[must_use]
pub fn webview_ok(path: &str) -> bool {
    hex_after(path, "/_ext/webview.", ".html").is_some()
}

/// Whether `folder` is an island's folder on the node.
fn folder_ok(folder: &str) -> bool {
    let Some(rest) = folder
        .strip_prefix("/_ext/island/")
        .and_then(|r| r.strip_suffix('/'))
    else {
        return false;
    };
    let Some((id, hash)) = rest.rsplit_once('.') else {
        return false;
    };
    !id.is_empty()
        && !id.starts_with('.')
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-'))
        && !hash.is_empty()
        && hash.chars().all(|c| c.is_ascii_hexdigit())
}

fn sheet_name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.split('/').all(|seg| {
            !seg.is_empty()
                && seg != "."
                && seg != ".."
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        })
}

fn text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty() && s.len() <= 64)
        .map(str::to_owned)
}

/// One listed island, if every part of it is what a web bundle must be
/// (the share page's `webOf`): a folder of the node's, a web view
/// document of the node's, one to 64 flat `web-*.js|wasm` files each with
/// a digest, the entry among them, exactly one wasm. Anything else: none.
fn island_of(v: &Value) -> Option<Island> {
    let folder = v.get("folder")?.as_str()?;
    let web = v.get("web")?;
    let entry = web.get("entry")?.as_str()?;
    let webview = web.get("webview")?.as_str()?;
    let listed = web.get("files")?.as_object()?;
    if !folder_ok(folder) || !webview_ok(webview) || listed.is_empty() || listed.len() > 64 {
        return None;
    }
    let mut files = BTreeMap::new();
    for (name, digest) in listed {
        let digest = digest.as_str()?;
        if !flat_web(name) || !pinned(digest) {
            return None;
        }
        files.insert(name.clone(), digest.to_owned());
    }
    if !files.contains_key(entry)
        || !ext_is(entry, "js")
        || files.keys().filter(|n| ext_is(n, "wasm")).count() != 1
    {
        return None;
    }
    let sheet = match (
        v.get("stylesheet").and_then(Value::as_str),
        v.get("stylesheetIntegrity").and_then(Value::as_str),
    ) {
        (Some(name), Some(digest)) if sheet_name_ok(name) && pinned(digest) => {
            Some((name.to_owned(), digest.to_owned()))
        }
        _ => None,
    };
    Some(Island {
        id: text(v, "id")?,
        folder: folder.to_owned(),
        webview: webview.to_owned(),
        entry: entry.to_owned(),
        files,
        sheet,
        progress: text(v, "progress"),
        transport: text(v, "transport"),
        lyrics: text(v, "lyrics"),
        embeds: embeds_of(v),
        reads: names_of(v.get("reads"), 8, name_ok),
    })
}

/// A view's or a capability's name: 1 to 64 of `[A-Za-z0-9._-]`.
fn name_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// A note type or a property's name as a claim lists it: 1 to 64
/// characters, not blank, no control character (the rule a type's
/// favourites have: anything a frontmatter `type:` can say).
fn key_ok(s: &str) -> bool {
    !s.trim().is_empty() && s.chars().count() <= 64 && !s.chars().any(char::is_control)
}

/// The strings of the array `v` that pass `ok`, at most `max`.
fn names_of(v: Option<&Value>, max: usize, ok: fn(&str) -> bool) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .filter(|s| ok(s))
                .map(str::to_owned)
                .take(max)
                .collect()
        })
        .unwrap_or_default()
}

/// An island's listed note embeds, at most 16: each a view name, one to
/// 16 note types, what it reads and up to 32 properties it declares; a
/// malformed one is left out.
fn embeds_of(v: &Value) -> Vec<Embed> {
    v.get("embeds")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(|e| {
                    let view = e.get("view")?.as_str().filter(|s| name_ok(s))?;
                    let types = names_of(e.get("note_type"), 16, key_ok);
                    (!types.is_empty()).then(|| Embed {
                        view: view.to_owned(),
                        note_types: types,
                        reads_note: names_of(e.get("reads"), 8, name_ok)
                            .iter()
                            .any(|r| r == READS_NOTE),
                        properties: names_of(e.get("properties"), 32, key_ok),
                    })
                })
                .take(16)
                .collect()
        })
        .unwrap_or_default()
}

/// The islands of the node's listing ([`LISTING`]) this host believes: a
/// malformed one is left out, never trusted in part.
#[must_use]
pub fn islands(listing: &Value) -> Vec<Island> {
    listing
        .get("islands")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(island_of).collect())
        .unwrap_or_default()
}

/// `sha256-<base64>` of `bytes`.
#[must_use]
pub fn sha256(bytes: &[u8]) -> String {
    use base64::Engine as _;
    use sha2::Digest as _;
    format!(
        "sha256-{}",
        base64::engine::general_purpose::STANDARD.encode(sha2::Sha256::digest(bytes))
    )
}

/// `bytes` are file `name` as pinned by `digest`, or why not.
///
/// # Errors
///
/// The bytes are not the file the listing pinned.
pub fn check(name: &str, bytes: &[u8], digest: &str) -> Result<(), String> {
    if sha256(bytes) == digest {
        Ok(())
    } else {
        Err(format!("{name} is not the file pinned"))
    }
}

/// The asks a frame may make: a token bucket, [`ASKS_PER_SECOND`] with a
/// burst of twice that (the share page's `INTENTS_PER_SECOND`).
#[derive(Debug, Clone, Copy)]
pub struct Bucket {
    tokens: f64,
    at: f64,
    rate: f64,
}

impl Bucket {
    /// One frame's, full at `now` (ms).
    #[must_use]
    pub const fn new(now: f64) -> Self {
        Self::at_rate(now, ASKS_PER_SECOND)
    }

    /// Every frame's together ([`ALL_ASKS_PER_SECOND`]), full at `now`.
    #[must_use]
    pub const fn all(now: f64) -> Self {
        Self::at_rate(now, ALL_ASKS_PER_SECOND)
    }

    const fn at_rate(now: f64, rate: f64) -> Self {
        Self {
            tokens: rate * 2.0,
            at: now,
            rate,
        }
    }

    fn fill(&mut self, now: f64) {
        let elapsed = (now - self.at).max(0.0) / 1000.0;
        self.tokens = elapsed.mul_add(self.rate, self.tokens).min(self.rate * 2.0);
        self.at = now;
    }

    /// Whether an ask at `now` (ms) is let through.
    pub fn take(&mut self, now: f64) -> bool {
        self.fill(now);
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
}

/// Whether a frame's ask at `now` is let through: its own bucket, then
/// every frame's. One frame that floods spends only its own, so the
/// others' asks still pass; together they never pass the shared bound.
pub fn take_ask(frame: &mut Bucket, all: &mut Bucket, now: f64) -> bool {
    if !frame.take(now) {
        return false;
    }
    if all.take(now) {
        return true;
    }
    // Refused by the shared bound: the frame's token is given back.
    frame.tokens = (frame.tokens + 1.0).min(frame.rate * 2.0);
    false
}

/// Why a frame's ask is not heard now, if it is not: before its view is
/// mounted (`ready`) or while the page is hidden, no ask is taken, so a
/// frame starts nothing the person cannot see it start.
#[must_use]
pub const fn ask_refused(ready: bool, hidden: bool) -> Option<&'static str> {
    if !ready {
        Some("not ready")
    } else if hidden {
        Some("the page is hidden")
    } else {
        None
    }
}

/// Whether `p` is a vault-relative path, as `architect.playhead` takes
/// `media` and `art`: the deck's one rule, shared with the frame host.
pub use architect_playhead::model::vault_path;

/// What a frame said, read from its message (JSON of at most
/// [`MAX_MESSAGE`] bytes); anything else is [`Said::Nothing`].
#[derive(Debug, Clone, PartialEq)]
pub enum Said {
    /// Its port is taken: boot it.
    Hello,
    /// Its view is mounted.
    Ready,
    /// The answer to ping `n`.
    Pong(u64),
    /// Its height, px.
    Size(f64),
    /// An ask.
    Ask(String, Value),
    /// It failed.
    Error(String),
    /// Not a message of the protocol.
    Nothing,
}

/// Reads a frame's message.
#[must_use]
pub fn said(json: &str) -> Said {
    if json.len() > MAX_MESSAGE {
        return Said::Nothing;
    }
    let Ok(v) = serde_json::from_str::<Value>(json) else {
        return Said::Nothing;
    };
    match v.get("t").and_then(Value::as_str) {
        Some("hello") => Said::Hello,
        Some("ready") => Said::Ready,
        Some("pong") => v
            .get("n")
            .and_then(Value::as_u64)
            .map_or(Said::Nothing, Said::Pong),
        Some("size") => v
            .get("height")
            .and_then(Value::as_f64)
            .filter(|h| h.is_finite())
            .map_or(Said::Nothing, |h| {
                Said::Size(h.round().clamp(0.0, MAX_HEIGHT))
            }),
        Some("ask") => match v.get("action").and_then(Value::as_str) {
            Some(a) if a.len() <= 64 => {
                Said::Ask(a.to_owned(), v.get("args").cloned().unwrap_or(Value::Null))
            }
            _ => Said::Nothing,
        },
        Some("error") => Said::Error(
            v.get("message")
                .and_then(Value::as_str)
                .unwrap_or("it failed")
                .chars()
                .take(200)
                .collect(),
        ),
        _ => Said::Nothing,
    }
}

/// The heartbeat of a ready frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct Beat {
    sent: u64,
    answered: u64,
    at: f64,
}

/// What the heartbeat does on a tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tick {
    /// Send ping `n`.
    Ping(u64),
    /// A ping is out and not late yet.
    Wait,
    /// A ping went unanswered past [`LATE_MS`].
    Dead,
}

impl Beat {
    /// A tick at `now` (ms).
    pub fn tick(&mut self, now: f64) -> Tick {
        if self.sent > self.answered {
            return if now - self.at > LATE_MS {
                Tick::Dead
            } else {
                Tick::Wait
            };
        }
        self.sent = self.sent.saturating_add(1);
        self.at = now;
        Tick::Ping(self.sent)
    }

    /// A `pong` with `n` at `now`: its round trip (ms) when it answers the
    /// ping out.
    pub fn pong(&mut self, n: u64, now: f64) -> Option<f64> {
        (n == self.sent && self.sent > self.answered).then(|| {
            self.answered = n;
            now - self.at
        })
    }

    /// The page was hidden and is shown again: what was not heard while
    /// hidden is not a sign of death.
    pub const fn shown(&mut self, now: f64) {
        self.answered = self.sent;
        self.at = now;
    }
}

/// Theme variables a frame takes: `--name` keys, values with no URL, no
/// markup and no `expression`, at most 200 characters.
#[must_use]
pub fn theme_vars<'a>(
    vars: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> BTreeMap<String, String> {
    vars.into_iter()
        .filter(|(k, v)| {
            k.len() <= 66
                && k.strip_prefix("--").is_some_and(|n| {
                    !n.is_empty()
                        && n.chars()
                            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
                })
                && !v.is_empty()
                && v.len() <= 200
                && !v.contains(['<', '>'])
                && !v.to_ascii_lowercase().contains("url(")
                && !v.to_ascii_lowercase().contains("expression")
        })
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .take(400)
        .collect()
}

#[cfg(test)]
// The helpers take JSON by value so a case reads as a literal.
#[allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::needless_pass_by_value
)]
mod tests {
    use serde_json::json;

    use super::*;

    fn d(b: &[u8]) -> String {
        sha256(b)
    }

    fn listing() -> Value {
        json!({
            "webview": "/_ext/webview.0123abcd.html",
            "islands": [{
                "id": "studio.transport",
                "folder": "/_ext/island/studio.transport.89abcdef/",
                "progress": "progress", "transport": "transport", "lyrics": "lyrics",
                "stylesheet": "island.css", "stylesheetIntegrity": d(b".x{}"),
                "web": {
                    "entry": "web-studio-island.js",
                    "webview": "/_ext/webview.0123abcd.html",
                    "files": {
                        "web-studio-island.js": d(b"glue"),
                        "web-studio-island_bg.wasm": d(b"wasm"),
                        "web-snippet-1.js": d(b"snip"),
                    }
                }
            }]
        })
    }

    #[test]
    fn a_note_type_draws_the_view_that_claims_it() {
        let mut l = listing();
        l["islands"][0]["embeds"] = json!([
            {"view": "list", "note_type": ["Playlist", "", "bad\ntype"]},
            {"view": "../x", "note_type": ["other"]},
            {"view": "none", "note_type": []},
            {"view": "second", "note_type": ["playlist", "a type with spaces"]},
        ]);
        let it = &islands(&l)[0];
        assert_eq!(it.embeds.len(), 2);
        assert_eq!(it.embeds[0].note_types, ["Playlist"]);
        // Within an island, the first claim listed wins.
        assert_eq!(it.embed_for("playlist"), Some("list"));
        assert_eq!(it.embed_for(" PLAYLIST "), Some("list"));
        // A type with a space is a type, as a type's favourites have it.
        assert_eq!(it.embed_for("a type with spaces"), Some("second"));
        assert_eq!(it.embed_for("other"), None);
        assert_eq!(it.embed_for(""), None);
        // An island that lists none claims none.
        assert!(islands(&listing())[0].embeds.is_empty());
    }

    /// Two islands: `first`, listed first, and `second`, with their claims.
    fn two(first: Value, second: Value) -> Vec<Island> {
        let mut l = listing();
        let mut b = l["islands"][0].clone();
        b["id"] = json!("other.ext");
        b["folder"] = json!("/_ext/island/other.ext.01234567/");
        l["islands"][0]["embeds"] = first;
        b["embeds"] = second;
        l["islands"].as_array_mut().unwrap().push(b);
        islands(&l)
    }

    #[test]
    fn the_island_that_draws_a_type_is_chosen_from_the_listing() {
        // The first island claims nothing: the second's claim is found.
        let list = two(
            json!([]),
            json!([{"view": "list", "note_type": ["playlist"]}]),
        );
        let (island, claim) = claimed(&list, "playlist").unwrap();
        assert_eq!(island.id, "other.ext");
        assert_eq!(claim.view, "list");
        // Both claim it: the first island listed wins, whatever the
        // second's claim says.
        let list = two(
            json!([{"view": "a", "note_type": ["poem"]}, {"view": "b", "note_type": ["playlist"]}]),
            json!([{"view": "c", "note_type": ["playlist"]}]),
        );
        let (island, claim) = claimed(&list, "Playlist").unwrap();
        assert_eq!(
            (island.id.as_str(), claim.view.as_str()),
            ("studio.transport", "b")
        );
        // A type nobody claims: no island, so nothing is fetched.
        assert!(claimed(&list, "song").is_none());
        assert!(claimed(&list, "").is_none());
        assert!(claimed(&[], "playlist").is_none());
    }

    #[test]
    fn a_listing_holds_at_most_sixteen_claims_of_sixteen_types() {
        let mut l = listing();
        let types: Vec<String> = (0..20).map(|n| format!("t{n}")).collect();
        let claims: Vec<Value> = (0..20)
            .map(|n| json!({"view": format!("v{n}"), "note_type": types, "properties": (0..40).map(|p| format!("p{p}")).collect::<Vec<_>>()}))
            .collect();
        l["islands"][0]["embeds"] = Value::Array(claims);
        l["islands"][0]["reads"] =
            json!(["documents", "a", "b", "c", "d", "e", "f", "g", "h", "i"]);
        let it = &islands(&l)[0];
        assert_eq!(it.embeds.len(), 16);
        assert_eq!(it.embeds[0].note_types.len(), 16);
        assert_eq!(it.embed_for("t15"), Some("v0"));
        assert_eq!(it.embed_for("t16"), None);
        assert_eq!(it.embeds[0].properties.len(), 32);
        assert_eq!(it.reads.len(), 8);
        // A view or a type past 64 characters is no claim.
        l["islands"][0]["embeds"] = json!([
            {"view": "v".repeat(65), "note_type": ["a"]},
            {"view": "v", "note_type": ["t".repeat(65)]},
        ]);
        assert!(islands(&l)[0].embeds.is_empty());
    }

    fn note_world(reads: &[&str], claim: Value) -> Island {
        let mut l = listing();
        l["islands"][0]["reads"] = json!(reads);
        l["islands"][0]["embeds"] = json!([claim]);
        islands(&l).remove(0)
    }

    const PROPS: &[(&str, &str)] = &[
        ("type", "playlist"),
        ("date", "2026-12-24"),
        ("private", "the alarm code"),
    ];

    #[test]
    fn a_view_that_declares_files_is_handed_them_checked_and_capped() {
        use architect_spaces::view::{VIEW_FILE_MAX, VIEW_FILES_MAX};
        let properties: Vec<(String, String)> = vec![("type".into(), "playlist".into())];
        let art = ViewFile::of("Art/Joy.png", b"\x89PNG").unwrap();
        let lrc = ViewFile::of("Songs/Joy.lrc", b"[00:01.00]Joy").unwrap();
        let files = vec![art.clone(), lrc.clone()];
        let note = Note {
            files: &files,
            ..the_note(&properties)
        };
        let claim = json!({"view": "list", "note_type": ["playlist"], "reads": ["documents"]});
        for audience in [Audience::Member, Audience::Guest] {
            let with = note_props(
                audience,
                &note_world(&["documents", "files"], claim.clone()),
                &note,
            );
            assert_eq!(
                with["files"],
                json!([
                    {"path": "Art/Joy.png", "kind": "image", "size": 4, "data": "data:image/png;base64,iVBORw=="},
                    {"path": "Songs/Joy.lrc", "kind": "lyrics", "size": 13, "text": "[00:01.00]Joy"},
                ]),
                "{audience:?}"
            );
            // Not declared: no files, though the caller had some.
            let without = note_props(audience, &note_world(&["documents"], claim.clone()), &note);
            assert!(without.get("files").is_none(), "{audience:?}");
            // `files` alone is no read of the note: nothing at all.
            let alone = note_props(audience, &note_world(&["files"], claim.clone()), &note);
            assert_eq!(alone, Value::Null);
        }
        // What a view never reads, a path a vault never holds, too large.
        assert!(ViewFile::of("Songs/Joy.mp3", b"x").is_err());
        assert!(ViewFile::of("Songs/Joy.svg", b"<svg/>").is_err());
        for bad in [
            "../Joy.png",
            "/etc/Joy.png",
            "a/%2e%2e/Joy.png",
            "a\\Joy.png",
            "C:/Joy.png",
            "a/./Joy.png",
        ] {
            assert!(ViewFile::of(bad, b"x").is_err(), "{bad}");
        }
        let big = vec![0_u8; usize::try_from(VIEW_FILE_MAX).unwrap() + 1];
        assert!(ViewFile::of("Art/Big.png", &big).is_err());
        assert!(ViewFile::of("Songs/Bad.lrc", &[0xff, 0xfe]).is_err());
        // A file whose kind was changed after its check, a path that is
        // not a vault's: left out by the gate itself.
        let forged = vec![
            ViewFile {
                kind: architect_spaces::view::ViewFileKind::Lyrics,
                ..art
            },
            ViewFile {
                path: "../x.lrc".into(),
                ..lrc.clone()
            },
        ];
        let note = Note {
            files: &forged,
            ..the_note(&properties)
        };
        let got = note_props(
            Audience::Guest,
            &note_world(&["documents", "files"], claim.clone()),
            &note,
        );
        assert_eq!(got["files"], json!([]));
        // At most the count cap, whatever the caller offers.
        let many: Vec<ViewFile> = (0..100)
            .map(|i| ViewFile {
                path: format!("L/{i}.lrc"),
                ..lrc.clone()
            })
            .collect();
        let note = Note {
            files: &many,
            ..the_note(&properties)
        };
        let got = note_props(
            Audience::Member,
            &note_world(&["documents", "files"], claim),
            &note,
        );
        assert_eq!(got["files"].as_array().map(Vec::len), Some(VIEW_FILES_MAX));
    }

    fn the_note(properties: &[(String, String)]) -> Note<'_> {
        Note {
            path: "Private/Folders/Merry.md",
            title: "Merry",
            note_type: "playlist",
            properties,
            body: "One %%a private comment%% two\n%%across\nlines%%three %%never closed",
            files: &[],
        }
    }

    #[test]
    fn a_view_is_handed_only_what_the_gate_lets_through_for_who_is_viewing() {
        let properties: Vec<(String, String)> = PROPS
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        let note = the_note(&properties);
        let reading = json!({"view": "list", "note_type": ["playlist"], "reads": ["documents"], "properties": ["date"]});
        let island = note_world(&["documents"], reading.clone());
        // A member: the note, its comments gone.
        let member = note_props(Audience::Member, &island, &note);
        assert_eq!(member["path"], "Private/Folders/Merry.md");
        assert_eq!(member["body"], "One  two\nthree ");
        assert_eq!(member["properties"]["private"], "the alarm code");
        assert!(!member.to_string().contains("comment"));
        // A guest (#648, #669): title, type, the declared properties and
        // the body the node gave, its comments gone too; no path, nothing
        // undeclared, and no files (its extension declares no `files`).
        let guest = note_props(Audience::Guest, &island, &note);
        assert_eq!(
            guest,
            json!({"title": "Merry", "type": "playlist", "properties": {"date": "2026-12-24"},
                   "body": "One  two\nthree "})
        );
        // The claim names no read, or the extension does not declare it,
        // or nothing claims the type: no note at all.
        let silent = json!({"view": "list", "note_type": ["playlist"], "properties": ["date"]});
        for island in [
            note_world(&["documents"], silent),
            note_world(&[], reading.clone()),
            note_world(&["selection"], reading),
            note_world(
                &["documents"],
                json!({"view": "list", "note_type": ["poem"], "reads": ["documents"]}),
            ),
        ] {
            assert_eq!(note_props(Audience::Member, &island, &note), Value::Null);
            assert_eq!(note_props(Audience::Guest, &island, &note), Value::Null);
        }
        assert_eq!(without_comments("no comments"), "no comments");
        assert_eq!(without_comments("%%all%%"), "");
        // A `%%` in code is text (#659 re-check N1): the comment after it
        // is still hidden, and what is shown is not dropped.
        assert_eq!(
            without_comments("Type `%%` to start a comment. %%private plan%% Done."),
            "Type `%%` to start a comment.  Done."
        );
        assert_eq!(without_comments("``a ` %% b`` %%x%%y"), "``a ` %% b`` y");
        assert_eq!(without_comments("a ` %%x%% b"), "a `  b");
        assert_eq!(
            without_comments("```\n%% kept\n```\n%%gone%%after\n"),
            "```\n%% kept\n```\nafter\n"
        );
        assert_eq!(
            without_comments("~~~~md\n%%\n~~~\n%%\n~~~~\nz%%q"),
            "~~~~md\n%%\n~~~\n%%\n~~~~\nz"
        );
        assert_eq!(without_comments("a %%b\n```\nc\n```\nd%% e"), "a  e");
    }

    #[test]
    fn an_islands_views_are_every_view_it_lists_once() {
        let mut l = listing();
        l["islands"][0]["embeds"] = json!([
            {"view": "list", "note_type": ["playlist"]},
            {"view": "lyrics", "note_type": ["poem"]},
        ]);
        let it = &islands(&l)[0];
        assert_eq!(it.views(), ["progress", "transport", "lyrics", "list"]);
        l["islands"][0]["lyrics"] = Value::Null;
        l["islands"][0]["transport"] = Value::Null;
        assert_eq!(islands(&l)[0].views(), ["progress", "list", "lyrics"]);
    }

    #[test]
    fn a_listed_island_is_believed_whole_or_not_at_all() {
        let all = islands(&listing());
        assert_eq!(all.len(), 1);
        let it = &all[0];
        assert_eq!(it.entry, "web-studio-island.js");
        assert_eq!(it.files.len(), 3);
        assert_eq!(it.sheet.as_ref().unwrap().0, "island.css");
        assert_eq!(it.lyrics.as_deref(), Some("lyrics"));
        let broken = |f: &dyn Fn(&mut Value)| {
            let mut l = listing();
            f(&mut l["islands"][0]);
            islands(&l).is_empty()
        };
        // A file outside the folder, a nested one, an unpinned one.
        assert!(broken(
            &|i| i["web"]["files"]["../web-x.js"] = json!(d(b"x"))
        ));
        assert!(broken(&|i| i["web"]["files"]["a/web-x.js"] = json!(d(b"x"))));
        assert!(broken(
            &|i| i["web"]["files"]["web-x.js"] = json!("sha256-nope")
        ));
        assert!(broken(&|i| i["web"]["files"]["web-x.html"] = json!(d(b"x"))));
        // Two wasm modules; an entry not listed; a wasm entry.
        assert!(broken(
            &|i| i["web"]["files"]["web-y_bg.wasm"] = json!(d(b"y"))
        ));
        assert!(broken(&|i| i["web"]["entry"] = json!("web-other.js")));
        assert!(broken(
            &|i| i["web"]["entry"] = json!("web-studio-island_bg.wasm")
        ));
        // A frame document or folder that is not the node's.
        for bad in [
            "https://evil.test/_ext/webview.00.html",
            "/_ext/webview.html",
            "/_ext/webview.zz.html",
            "//evil.test/_ext/webview.00.html",
            "/s/x/webview.00.html",
            // WebRTC is the app's to grant, never the listing's (#614).
            "/_ext/webview-rtc.0123abcd.html",
            "javascript:alert(1)",
        ] {
            assert!(broken(&|i| i["web"]["webview"] = json!(bad)), "{bad}");
        }
        for bad in [
            "https://evil.test/_ext/island/x.00/",
            "/_ext/island/../x.00/",
            "/_ext/island/x/",
            "/_ext/island/x.00",
        ] {
            assert!(broken(&|i| i["folder"] = json!(bad)), "{bad}");
        }
        // No web bundle: nothing a frame could run.
        assert!(broken(&|i| i["web"] = Value::Null));
        // A sheet without its digest is no sheet; the island stays.
        let mut l = listing();
        l["islands"][0]["stylesheetIntegrity"] = Value::Null;
        assert!(islands(&l)[0].sheet.is_none());
        assert!(islands(&json!({})).is_empty());
    }

    #[test]
    fn each_file_is_checked_against_its_digest() {
        assert!(check("a", b"glue", &d(b"glue")).is_ok());
        assert_eq!(
            check("web-x.js", b"evil", &d(b"glue")).unwrap_err(),
            "web-x.js is not the file pinned"
        );
        // The same digest the node gives (`architect_sites::island`).
        assert_eq!(
            sha256(b""),
            "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
        );
    }

    #[test]
    fn asks_are_the_playheads_controls_and_rate_limited() {
        let mut b = Bucket::new(0.0);
        let burst = (0..100).filter(|_| b.take(0.0)).count();
        assert_eq!(burst, 16);
        assert!(!b.take(10.0));
        // A second later: eight more.
        let more = (0..100).filter(|_| b.take(1000.0)).count();
        assert_eq!(more, 8);
        // One frame flooding spends its own bucket, not the others' (#660
        // L2): a second frame's asks still pass.
        let mut all = Bucket::all(0.0);
        let (mut flood, mut calm) = (Bucket::new(0.0), Bucket::new(0.0));
        let flooded = (0..1000)
            .filter(|_| take_ask(&mut flood, &mut all, 0.0))
            .count();
        assert_eq!(flooded, 16);
        assert!((0..5).all(|_| take_ask(&mut calm, &mut all, 1.0)));
        // Many frames together never pass the shared bound.
        let mut all = Bucket::all(0.0);
        let passed: usize = (0..20)
            .map(|_| {
                let mut f = Bucket::new(0.0);
                (0..100).filter(|_| take_ask(&mut f, &mut all, 0.0)).count()
            })
            .sum();
        assert_eq!(passed, 64);
        // #656's five, and ITEM and SET; no reads, no switching sources,
        // nothing of a vault's.
        assert_eq!(INTENTS.len(), 7);
        for a in [
            "PLAYHEAD_ITEM",
            "PLAYHEAD_SET",
            "PLAYHEAD_SEEK",
            "PLAYHEAD_PLAY",
        ] {
            assert!(INTENTS.contains(&a), "{a}");
        }
        for a in [
            "PLAYHEAD_ACTIVATE",
            "PLAYHEAD_STATE",
            "PLAYHEAD_SOURCES",
            "VAULT_READ",
            "SHARE_CREATE",
        ] {
            assert!(!INTENTS.contains(&a), "{a}");
        }
        // Not before the view is mounted, not while the page is hidden.
        assert_eq!(ask_refused(false, false), Some("not ready"));
        assert_eq!(ask_refused(true, true), Some("the page is hidden"));
        assert_eq!(ask_refused(false, true), Some("not ready"));
        assert_eq!(ask_refused(true, false), None);
    }

    #[test]
    fn a_view_at_a_notes_head_scrolls_past_a_share_of_the_window() {
        // A phone, 390 x 844: a 500-row list (22000 px) is 506 px tall and
        // scrolls inside; a short view is as tall as it says.
        assert!((head_height(22_000.0, 844.0) - 506.4).abs() < 1e-9);
        assert!((head_height(120.0, 844.0) - 120.0).abs() < f64::EPSILON);
        // A short window still gives a view room.
        assert!((head_height(4_000.0, 200.0) - HEAD_MIN).abs() < f64::EPSILON);
        assert!(head_height(-5.0, 844.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_whole_queue_fits_one_message() {
        let queue: Vec<Value> = (0..architect_playhead::model::MAX_QUEUE)
            .map(|n| json!({"id": format!("Setlists/Merry.md#{n}"), "title": format!("Song number {n}"), "subtitle": "Traditional", "duration": 192.5}))
            .collect();
        let message = json!({"t": "ask", "action": "PLAYHEAD_SET", "args": {"name": "Studio", "queue": queue}}).to_string();
        assert!(
            message.len() > 16 * 1024 && message.len() <= MAX_MESSAGE,
            "{}",
            message.len()
        );
        let Said::Ask(action, args) = said(&message) else {
            panic!("a whole queue is heard")
        };
        assert_eq!(action, "PLAYHEAD_SET");
        assert!(architect_playhead::Spec::parse(&args).is_ok());
    }

    #[test]
    fn a_frames_messages_are_read_as_the_protocol_or_not_at_all() {
        assert_eq!(said(r#"{"t":"hello"}"#), Said::Hello);
        assert_eq!(said(r#"{"t":"ready"}"#), Said::Ready);
        assert_eq!(said(r#"{"t":"pong","n":3}"#), Said::Pong(3));
        assert_eq!(
            said(r#"{"t":"size","height":99999}"#),
            Said::Size(MAX_HEIGHT)
        );
        assert_eq!(said(r#"{"t":"size","height":-5}"#), Said::Size(0.0));
        assert_eq!(said(r#"{"t":"size","height":"x"}"#), Said::Nothing);
        assert_eq!(
            said(r#"{"t":"ask","action":"PLAYHEAD_PLAY"}"#),
            Said::Ask("PLAYHEAD_PLAY".to_owned(), Value::Null)
        );
        assert_eq!(said(r#"{"t":"boot"}"#), Said::Nothing);
        assert_eq!(said("not json"), Said::Nothing);
        let big = format!(
            r#"{{"t":"ask","action":"PLAYHEAD_PLAY","args":"{}"}}"#,
            "x".repeat(MAX_MESSAGE)
        );
        assert_eq!(said(&big), Said::Nothing);
    }

    #[test]
    fn the_heartbeat_finds_a_frame_that_stopped_answering() {
        let mut b = Beat::default();
        assert_eq!(b.tick(0.0), Tick::Ping(1));
        assert_eq!(b.pong(2, 5.0), None, "not the ping out");
        assert_eq!(b.pong(1, 5.0), Some(5.0));
        assert_eq!(b.pong(1, 6.0), None, "answered once");
        assert_eq!(b.tick(2000.0), Tick::Ping(2));
        assert_eq!(b.tick(4000.0), Tick::Wait);
        assert_eq!(b.tick(8001.0), Tick::Dead);
        // Hidden and shown again: a fresh start.
        b.shown(9000.0);
        assert_eq!(b.tick(9001.0), Tick::Ping(3));
    }

    #[test]
    fn the_theme_handed_in_carries_no_url_or_markup() {
        let vars = theme_vars([
            ("--foreground", "#111"),
            ("--bg", "url(https://evil.test/x)"),
            ("--x", "</style><script>"),
            ("color", "red"),
            ("--", "red"),
            ("--y", "expression(alert(1))"),
            ("--font", "Inter, system-ui"),
        ]);
        assert_eq!(vars.keys().collect::<Vec<_>>(), ["--font", "--foreground"]);
    }
}
