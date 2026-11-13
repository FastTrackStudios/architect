//! The extension frame host's decisions (#495, route A), with no browser.
//!
//! What the node's islands listing says and whether to believe it, each
//! file against its digest, the asks a frame may make and how often, the
//! heartbeat, the theme handed in, and a song note's timed lyrics. The
//! DOM half (the frames, their ports, the timers) is `app::frames`.
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
//!    [`INTENTS`], rate-limited by [`Bucket`]) or `error`.
//!
//! Besides its slots by a song (progress, transport, lyrics), an island
//! may claim note types ([`Embed`]): its view is drawn at the head of a
//! note of that `type` ([`claimed`]: the first island listed that claims
//! it), given what [`note_props`] lets through of the note for who is
//! viewing, and may set its source in the app's playhead (`PLAYHEAD_SET`)
//! and make one of its items current (`PLAYHEAD_ITEM`), as
//! `architect.playhead` v0.1 has them (#589).
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
/// At most this many asks a second, with a burst of twice that.
pub const ASKS_PER_SECOND: f64 = 8.0;
/// The most bytes a message from a frame may take as JSON; larger is
/// dropped unread (`architect.playhead` v0.1: a whole queue fits one
/// `PLAYHEAD_SET`).
pub const MAX_MESSAGE: usize = 64 * 1024;

/// What a frame may ask the app: the playhead's intents, nothing else
/// (what an island's `capabilities.actions` holds; host calls into a
/// vault are #495's next step, authorised in the app and the node).
pub const INTENTS: &[&str] = &[
    "PLAYHEAD_SEEK",
    "PLAYHEAD_PLAY",
    "PLAYHEAD_PAUSE",
    "PLAYHEAD_NEXT",
    "PLAYHEAD_PREVIOUS",
    // `architect.playhead` v0.1 (#589): its source, and one item of it.
    "PLAYHEAD_SET",
    "PLAYHEAD_ITEM",
];

/// The most items a source's queue may hold (#589).
pub const QUEUE_MAX: usize = 500;
/// The most sections an item may hold (#589).
pub const SECTIONS_MAX: usize = 200;
/// The most marks an item may hold (#589).
pub const MARKS_MAX: usize = 2000;

/// The read capability a claim names, and its extension declares
/// (`[capabilities] reads`), for its view to be handed the note it is
/// drawn on: the app's documents.
pub const READS_NOTE: &str = "documents";

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
    /// A share link's guest (#629). What a guest gets of a note's text is
    /// the owner's decision, still open (#648); until it is made, the
    /// conservative answer: its title, its type and the properties the
    /// claim declares for that type. No body, no vault path.
    Guest,
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
    /// Its markdown after the frontmatter.
    pub body: &'a str,
}

/// What `island`'s views are handed of `note` for `audience`.
///
/// The one gate between a vault's note and a frame, wherever a view is
/// mounted on a note. `null` unless the island claims the note's type with a claim
/// that names [`READS_NOTE`] and its extension declares it too.
///
/// - [`Audience::Member`]: `{path, title, type, properties, body}`, the
///   body without its `%%comments%%`;
/// - [`Audience::Guest`]: `{title, type, properties}`, only the properties
///   the claim declares.
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
    match audience {
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
        }),
    }
}

/// `body` without its `%%comments%%`.
///
/// A comment may span lines, and a `%%` never closed hides the rest. A
/// `%%` inside code is text, not a comment's edge: a fenced block (three
/// or more backticks or tildes, closed by as many) is kept whole, and so
/// is an inline code span on one line.
#[must_use]
pub fn without_comments(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut fence: Option<(char, usize)> = None;
    let mut hidden = false;
    for line in body.split_inclusive('\n') {
        if !hidden {
            if let Some((mark, len)) = fence {
                out.push_str(line);
                if fence_of(line).is_some_and(|(c, n, bare)| c == mark && n >= len && bare) {
                    fence = None;
                }
                continue;
            }
            if let Some((mark, len, _)) = fence_of(line) {
                fence = Some((mark, len));
                out.push_str(line);
                continue;
            }
        }
        hidden = uncommented(line, hidden, &mut out);
    }
    out
}

/// A line's code fence: its mark, how many, and whether nothing follows
/// them (what a closing fence is).
fn fence_of(line: &str) -> Option<(char, usize, bool)> {
    let text = line.trim_start_matches(' ');
    if line.len().saturating_sub(text.len()) > 3 {
        return None;
    }
    let mark = text.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let len = text.chars().take_while(|c| *c == mark).count();
    (len >= 3).then(|| (mark, len, text.trim_start_matches(mark).trim().is_empty()))
}

/// One line outside a fence onto `out` without its comments, given
/// whether it starts inside one; whether it ends inside one.
fn uncommented(line: &str, mut hidden: bool, out: &mut String) -> bool {
    let mut rest = line;
    loop {
        if hidden {
            let Some((_, tail)) = rest.split_once("%%") else {
                return true;
            };
            hidden = false;
            rest = tail;
            continue;
        }
        let Some(comment) = rest.find("%%") else {
            out.push_str(rest);
            return false;
        };
        if let Some(tick) = rest.find('`').filter(|tick| *tick < comment) {
            let (before, from) = rest.split_at_checked(tick).unwrap_or((rest, ""));
            out.push_str(before);
            let len = from.chars().take_while(|c| *c == '`').count();
            let (run, after) = from.split_at_checked(len).unwrap_or((from, ""));
            out.push_str(run);
            // A span is kept whole; a run never closed is only text.
            let (span, tail) = code_span_end(after, len)
                .and_then(|end| after.split_at_checked(end))
                .unwrap_or(("", after));
            out.push_str(span);
            rest = tail;
        } else {
            let (before, from) = rest.split_at_checked(comment).unwrap_or((rest, ""));
            out.push_str(before);
            rest = from.strip_prefix("%%").unwrap_or(from);
            hidden = true;
        }
    }
}

/// Where the code span opened by `len` backticks ends in `after`: just
/// past the next run of exactly `len`.
fn code_span_end(after: &str, len: usize) -> Option<usize> {
    let mut seen = 0_usize;
    let mut rest = after;
    loop {
        let tick = rest.find('`')?;
        let from = rest.get(tick..)?;
        let run = from.chars().take_while(|c| *c == '`').count();
        let next = tick.checked_add(run)?;
        seen = seen.checked_add(next)?;
        if run == len {
            return Some(seen);
        }
        rest = rest.get(next..)?;
    }
}

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
}

impl Bucket {
    /// Full at `now` (ms).
    #[must_use]
    pub const fn new(now: f64) -> Self {
        Self {
            tokens: ASKS_PER_SECOND * 2.0,
            at: now,
        }
    }

    /// Whether an ask at `now` (ms) is let through.
    pub fn take(&mut self, now: f64) -> bool {
        let elapsed = (now - self.at).max(0.0) / 1000.0;
        self.tokens = elapsed
            .mul_add(ASKS_PER_SECOND, self.tokens)
            .min(ASKS_PER_SECOND * 2.0);
        self.at = now;
        if self.tokens < 1.0 {
            return false;
        }
        self.tokens -= 1.0;
        true
    }
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

/// An ask the app answers: one of [`INTENTS`], with its arguments.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// To `t` seconds into the song.
    Seek(f64),
    /// Play.
    Play,
    /// Pause.
    Pause,
    /// The next song.
    Next,
    /// The previous song (or this one's start).
    Previous,
    /// Its source (`PLAYHEAD_SET`; an empty queue removes it).
    Set(Source),
    /// Make this item current, at 0, and play it if `play`
    /// (`PLAYHEAD_ITEM`).
    Item(Pick, bool),
}

/// Which item a `PLAYHEAD_ITEM` makes current.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    /// The item of this id.
    Id(String),
    /// The item at this place in the queue.
    Index(usize),
}

/// A source as `PLAYHEAD_SET` gives it (#589).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Source {
    /// Its name, shown as the source's (empty: the extension's).
    pub name: String,
    /// Its items.
    pub queue: Vec<QueueItem>,
    /// The item to make current (clamped to the queue), if given.
    pub index: Option<usize>,
    /// Where in it to start, seconds, if given.
    pub time: Option<f64>,
    /// Whether to start playing now.
    pub play: bool,
}

/// An item of a source's queue (#589), what this host keeps of it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueueItem {
    /// The extension's own id for it.
    pub id: String,
    pub title: String,
    pub subtitle: String,
    /// Seconds: given, or none (then its media's own length).
    pub duration: Option<f64>,
    /// A vault path to its audio.
    pub media: Option<String>,
    /// A vault path to its image.
    pub art: Option<String>,
    /// Its sections, checked against the contract's limits.
    pub sections: Vec<ItemSection>,
    /// Its marks (timed lyric lines and cues), checked likewise.
    pub marks: Vec<ItemMark>,
}

/// A section of a queue's item.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemSection {
    /// 1 to 40 characters.
    pub name: String,
    /// Its start, seconds.
    pub start: f64,
    /// Its end, seconds, not before its start.
    pub end: f64,
    /// `#rrggbb`.
    pub color: Option<String>,
}

/// A mark of a queue's item.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemMark {
    /// When, seconds.
    pub at: f64,
    /// 1 to 300 characters.
    pub text: String,
    /// At most 32 characters (`line`, `cue`).
    pub kind: Option<String>,
}

impl QueueItem {
    /// The item as a view's props carry it: what was given, checked.
    #[must_use]
    pub fn to_json(&self) -> Value {
        let sections: Vec<Value> = self
            .sections
            .iter()
            .map(|s| {
                serde_json::json!({"name": s.name, "start": s.start, "end": s.end, "color": s.color})
            })
            .collect();
        let marks: Vec<Value> = self
            .marks
            .iter()
            .map(|m| serde_json::json!({"at": m.at, "text": m.text, "kind": m.kind}))
            .collect();
        serde_json::json!({
            "id": self.id,
            "title": self.title,
            "subtitle": self.subtitle,
            "duration": self.duration,
            "media": self.media,
            "art": self.art,
            "sections": sections,
            "marks": marks,
        })
    }

    /// Its sections as a view's props carry them.
    #[must_use]
    pub fn sections_json(&self) -> Value {
        self.to_json()
            .get("sections")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()))
    }
}

/// `v[k]` as a string of 1 to `max` characters; `Err` for one too long or
/// not a string, `Ok(None)` for none.
fn field(v: &Value, k: &str, max: usize) -> Result<Option<String>, String> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.chars().count() <= max => {
            Ok(Some(s.clone()).filter(|s| !s.is_empty()))
        }
        Some(_) => Err(format!("{k} is at most {max} characters")),
    }
}

/// `v[k]` as a required string of 1 to `max` characters.
fn required(v: &Value, k: &str, max: usize) -> Result<String, String> {
    field(v, k, max)?.ok_or_else(|| format!("{k} is required"))
}

/// `v[k]` as seconds (finite, not negative), if given.
fn seconds(v: &Value, k: &str) -> Result<Option<f64>, String> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(n) => n
            .as_f64()
            .filter(|d| d.is_finite() && *d >= 0.0)
            .map(Some)
            .ok_or_else(|| format!("{k} is not a time in seconds")),
    }
}

/// A vault-relative path, as `architect.playhead` v0.1 takes `media` and
/// `art`.
///
/// 1 to 1024 bytes; no scheme or drive (`:`), not absolute (a
/// leading `/`), no `\`, no `?` or `#` (a query or fragment once it is a
/// URL), no `%` (percent-encoding), no control character, no empty, `.`
/// or `..` segment.
///
/// Whether it is a path the note references is the deck's to enforce
/// when it resolves the media (#589).
#[must_use]
pub fn vault_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= 1024
        && !p
            .chars()
            .any(|c| matches!(c, ':' | '\\' | '?' | '#' | '%') || c.is_control())
        && p.split('/').all(|s| !s.is_empty() && s != "." && s != "..")
}

fn color_ok(c: &str) -> bool {
    c.len() == 7
        && c.strip_prefix('#')
            .is_some_and(|h| h.chars().all(|c| c.is_ascii_hexdigit()))
}

fn section_of(v: &Value) -> Result<ItemSection, String> {
    let start = seconds(v, "start")?.ok_or("a section's start is required")?;
    let end = seconds(v, "end")?.ok_or("a section's end is required")?;
    let color = field(v, "color", 7)?;
    if end < start {
        return Err("a section ends before it starts".to_owned());
    }
    if color.as_deref().is_some_and(|c| !color_ok(c)) {
        return Err("a section's color is #rrggbb".to_owned());
    }
    Ok(ItemSection {
        name: required(v, "name", 40)?,
        start,
        end,
        color,
    })
}

fn mark_of(v: &Value) -> Result<ItemMark, String> {
    Ok(ItemMark {
        at: seconds(v, "at")?.ok_or("a mark's at is required")?,
        text: required(v, "text", 300)?,
        kind: field(v, "kind", 32)?,
    })
}

/// `v[k]`, an array of at most `max`, each read by `one`.
fn list_of<T>(
    v: &Value,
    k: &str,
    max: usize,
    one: fn(&Value) -> Result<T, String>,
) -> Result<Vec<T>, String> {
    match v.get(k) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(list)) if list.len() <= max => list.iter().map(one).collect(),
        Some(Value::Array(_)) => Err(format!("{k}: at most {max}")),
        Some(_) => Err(format!("{k} is a list")),
    }
}

fn item_of(i: &Value) -> Result<QueueItem, String> {
    let id = field(i, "id", 256)?.ok_or("an item's id is 1 to 256 characters")?;
    let media = field(i, "media", 1024)?;
    let art = field(i, "art", 1024)?;
    if media.as_deref().is_some_and(|m| !vault_path(m)) {
        return Err("media must be a vault path".to_owned());
    }
    if art.as_deref().is_some_and(|a| !vault_path(a)) {
        return Err("art must be a vault path".to_owned());
    }
    let duration = match i.get("duration") {
        None | Some(Value::Null) => None,
        Some(d) => Some(
            d.as_f64()
                .filter(|d| d.is_finite() && *d > 0.0)
                .ok_or("duration is a length in seconds")?,
        ),
    };
    if media.is_none() && duration.is_none() {
        return Err("an item without media needs a duration".to_owned());
    }
    Ok(QueueItem {
        title: required(i, "title", 200)?,
        subtitle: field(i, "subtitle", 200)?.unwrap_or_default(),
        sections: list_of(i, "sections", SECTIONS_MAX, section_of)?,
        marks: list_of(i, "marks", MARKS_MAX, mark_of)?,
        id,
        duration,
        media,
        art,
    })
}

/// A `PLAYHEAD_SET`'s source, as #589 v0.1 bounds it: a `name` of at most
/// 64 characters, at most [`QUEUE_MAX`] items, each with a unique `id` (1
/// to 256), a `title` (1 to 200), a `subtitle` (at most 200), `media` and
/// `art` vault paths ([`vault_path`]), a `duration` when it has no
/// `media`, at most [`SECTIONS_MAX`] sections and [`MARKS_MAX`] marks;
/// `index`, `time` and `play` as given. Anything else refuses the whole
/// source, with why.
fn source_of(args: &Value) -> Result<Source, String> {
    let name = field(args, "name", 64)?.unwrap_or_default();
    let items = args
        .get("queue")
        .and_then(Value::as_array)
        .ok_or("queue is required")?;
    if items.len() > QUEUE_MAX {
        return Err(format!("queue: at most {QUEUE_MAX} items"));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut queue = Vec::with_capacity(items.len());
    for i in items {
        let item = item_of(i)?;
        if !ids.insert(item.id.clone()) {
            return Err(format!("item {} is in the queue twice", item.id));
        }
        queue.push(item);
    }
    let index = match args.get("index") {
        None | Some(Value::Null) => None,
        Some(n) => Some(
            n.as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("index is a place in the queue")?,
        ),
    };
    Ok(Source {
        name,
        queue,
        index,
        time: seconds(args, "time")?,
        play: args.get("play").and_then(Value::as_bool).unwrap_or(false),
    })
}

/// `action` with `args` as an [`Intent`], if it is one and its arguments
/// are what it takes (a seek within `0..=duration`), asked by a frame of
/// the extension `caller`.
///
/// # Errors
///
/// Why it is refused, as `architect.playhead` v0.1 words it: not one of
/// [`INTENTS`], `"not your source"` for a `source` naming another
/// extension, `"t is outside 0..=duration"`, a field's limit.
pub fn intent(action: &str, args: &Value, duration: f64, caller: &str) -> Result<Intent, String> {
    if !INTENTS.contains(&action) {
        return Err(format!("{action} is not something a view may ask"));
    }
    // A frame acts on its own extension's source only.
    match args.get("source") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s == caller => {}
        Some(_) => return Err("not your source".to_owned()),
    }
    match action {
        "PLAYHEAD_SEEK" => {
            let t = args
                .get("t")
                .and_then(Value::as_f64)
                .ok_or("t is required")?;
            (t.is_finite() && t >= 0.0 && duration > 0.0 && t <= duration)
                .then_some(Intent::Seek(t))
                .ok_or_else(|| "t is outside 0..=duration".to_owned())
        }
        "PLAYHEAD_PLAY" => Ok(Intent::Play),
        "PLAYHEAD_PAUSE" => Ok(Intent::Pause),
        "PLAYHEAD_NEXT" => Ok(Intent::Next),
        "PLAYHEAD_PREVIOUS" => Ok(Intent::Previous),
        "PLAYHEAD_SET" => source_of(args).map(Intent::Set),
        "PLAYHEAD_ITEM" => {
            let pick = match (field(args, "id", 256)?, args.get("index")) {
                (Some(id), _) => Pick::Id(id),
                (None, Some(n)) => Pick::Index(
                    n.as_u64()
                        .and_then(|n| usize::try_from(n).ok())
                        .ok_or("index is a place in the queue")?,
                ),
                (None, None) => return Err("an id or an index is required".to_owned()),
            };
            Ok(Intent::Item(
                pick,
                args.get("play").and_then(Value::as_bool).unwrap_or(false),
            ))
        }
        _ => Err(format!("{action} is not something a view may ask")),
    }
}

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

/// A timed line of a song's lyrics.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// When, seconds.
    pub at: f64,
    /// The words.
    pub text: String,
}

/// A song's section, from its lyrics' `[Name]` lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Its name.
    pub name: String,
    /// Its start, seconds.
    pub start: f64,
    /// Its end, seconds.
    pub end: f64,
}

/// `[mm:ss.xx]` as seconds.
fn stamp(s: &str) -> Option<f64> {
    let (m, rest) = s.split_once(':')?;
    let m: f64 = m.trim().parse().ok()?;
    let sec: f64 = rest.trim().parse().ok()?;
    (m >= 0.0 && (0.0..60.0).contains(&sec)).then_some(m.mul_add(60.0, sec))
}

/// A note body's timed lyrics: its first ` ```lrc ` fence, a line each
/// `[mm:ss.xx] words`; a line whose words are `[Name]` starts section
/// `Name` (ending where the next starts, the last at `duration`).
#[must_use]
pub fn lyrics(body: &str, duration: f64) -> (Vec<Line>, Vec<Section>) {
    let mut lines = Vec::new();
    let mut marks: Vec<(String, f64)> = Vec::new();
    let mut inside = false;
    for raw in body.lines() {
        let l = raw.trim();
        if !inside {
            inside = l == "```lrc";
            continue;
        }
        if l.starts_with("```") {
            break;
        }
        let Some((at, words)) = l
            .strip_prefix('[')
            .and_then(|r| r.split_once(']'))
            .and_then(|(t, w)| stamp(t).map(|at| (at, w.trim())))
        else {
            continue;
        };
        if let Some(name) = words.strip_prefix('[').and_then(|w| w.strip_suffix(']')) {
            marks.push((name.chars().take(40).collect(), at));
        } else if !words.is_empty() && lines.len() < 2000 {
            lines.push(Line {
                at,
                text: words.chars().take(300).collect(),
            });
        }
    }
    lines.sort_by(|a, b| a.at.total_cmp(&b.at));
    marks.sort_by(|a, b| a.1.total_cmp(&b.1));
    let sections = marks
        .iter()
        .enumerate()
        .map(|(i, (name, start))| Section {
            name: name.clone(),
            start: *start,
            end: marks
                .get(i.saturating_add(1))
                .map_or(duration, |m| m.1)
                .max(*start),
        })
        .collect();
    (lines, sections)
}

/// A song note's length in seconds: its `duration` property (seconds, or
/// `m:ss`), else ten seconds past its last timed line, else three minutes.
#[must_use]
pub fn duration(property: Option<&str>, lines: &[Line]) -> f64 {
    property
        .and_then(|p| p.trim().parse::<f64>().ok().or_else(|| stamp(p)))
        .filter(|d| d.is_finite() && *d > 0.0)
        .unwrap_or_else(|| lines.last().map_or(180.0, |l| l.at + 10.0))
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

    fn the_note(properties: &[(String, String)]) -> Note<'_> {
        Note {
            path: "Private/Folders/Merry.md",
            title: "Merry",
            note_type: "playlist",
            properties,
            body: "One %%a private comment%% two\n%%across\nlines%%three %%never closed",
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
        // A guest: title, type and the declared properties; no body, no
        // path, nothing undeclared.
        let guest = note_props(Audience::Guest, &island, &note);
        assert_eq!(
            guest,
            json!({"title": "Merry", "type": "playlist", "properties": {"date": "2026-12-24"}})
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

    fn ask(action: &str, args: Value) -> Result<Intent, String> {
        intent(action, &args, 10.0, "studio.transport")
    }

    fn set(queue: Value) -> Result<Intent, String> {
        ask("PLAYHEAD_SET", json!({"name": "N", "queue": queue}))
    }

    fn one(item: Value) -> Result<Intent, String> {
        set(json!([item]))
    }

    #[test]
    fn asks_are_the_playheads_intents_and_rate_limited() {
        let mut b = Bucket::new(0.0);
        let burst = (0..100).filter(|_| b.take(0.0)).count();
        assert_eq!(burst, 16);
        assert!(!b.take(10.0));
        // A second later: eight more.
        let more = (0..100).filter(|_| b.take(1000.0)).count();
        assert_eq!(more, 8);
        assert_eq!(
            ask("PLAYHEAD_SEEK", json!({"t": 3.5})),
            Ok(Intent::Seek(3.5))
        );
        for t in [json!(30), json!(-1)] {
            assert_eq!(
                ask("PLAYHEAD_SEEK", json!({ "t": t })).unwrap_err(),
                "t is outside 0..=duration"
            );
        }
        assert!(ask("PLAYHEAD_SEEK", json!({})).is_err());
        assert_eq!(ask("PLAYHEAD_PLAY", Value::Null), Ok(Intent::Play));
        assert!(ask("VAULT_READ", json!({"path": "a"})).is_err());
        assert!(ask("SHARE_CREATE", Value::Null).is_err());
        // Only its own source (#589).
        assert_eq!(
            ask("PLAYHEAD_PLAY", json!({"source": "other.ext"})).unwrap_err(),
            "not your source"
        );
        assert_eq!(
            ask("PLAYHEAD_PLAY", json!({"source": "studio.transport"})),
            Ok(Intent::Play)
        );
        let args = json!({"t": 1, "id": "a", "queue": [{"id": "a", "title": "A", "duration": 2}]});
        assert!(INTENTS.iter().all(|a| intent(a, &args, 10.0, "x").is_ok()));
        // Not before the view is mounted, not while the page is hidden.
        assert_eq!(ask_refused(false, false), Some("not ready"));
        assert_eq!(ask_refused(true, true), Some("the page is hidden"));
        assert_eq!(ask_refused(false, true), Some("not ready"));
        assert_eq!(ask_refused(true, false), None);
    }

    #[test]
    fn a_source_is_what_the_contract_bounds_or_refused_whole() {
        let item = |extra: Value| {
            let mut i = json!({"id": "a", "title": "A", "duration": 2});
            for (k, v) in extra.as_object().unwrap() {
                i[k] = v.clone();
            }
            one(i)
        };
        let Ok(Intent::Set(source)) = ask(
            "PLAYHEAD_SET",
            json!({"name": "N", "index": 1, "time": 4.5, "play": true, "queue": [
                {"id": "a", "title": "A", "subtitle": "S", "media": "Media/a.ogg", "art": "Art/a b.jpg",
                 "sections": [{"name": "Verse 1", "start": 0, "end": 20.5, "color": "#2563eb"}],
                 "marks": [{"at": 1.5, "text": "Amazing grace", "kind": "line"}]},
                {"id": "b", "title": "B", "duration": 3},
            ]}),
        ) else {
            panic!("a good source")
        };
        assert_eq!(
            (source.index, source.time, source.play),
            (Some(1), Some(4.5), true)
        );
        let a = &source.queue[0];
        assert_eq!(a.media.as_deref(), Some("Media/a.ogg"));
        assert_eq!(a.to_json()["art"], "Art/a b.jpg");
        assert_eq!(a.to_json()["sections"][0]["color"], "#2563eb");
        assert_eq!(a.to_json()["marks"][0]["kind"], "line");
        assert_eq!(a.sections_json()[0]["name"], "Verse 1");
        assert_eq!(
            set(json!([])),
            Ok(Intent::Set(Source {
                name: "N".to_owned(),
                ..Source::default()
            }))
        );
        // Each limit holds at its edge and refuses one past it.
        let at = |n: usize| "x".repeat(n);
        for (k, max) in [("id", 256), ("title", 200), ("subtitle", 200)] {
            assert!(item(json!({ k: at(max) })).is_ok(), "{k} of {max}");
            assert!(
                item(json!({ k: at(max + 1) })).is_err(),
                "{k} of {}",
                max + 1
            );
        }
        assert!(ask("PLAYHEAD_SET", json!({"name": at(64), "queue": []})).is_ok());
        assert!(ask("PLAYHEAD_SET", json!({"name": at(65), "queue": []})).is_err());
        let sections = |n: usize| Value::Array(vec![json!({"name": "S", "start": 0, "end": 1}); n]);
        assert!(item(json!({"sections": sections(SECTIONS_MAX)})).is_ok());
        assert!(item(json!({"sections": sections(SECTIONS_MAX + 1)})).is_err());
        let marks = |n: usize| Value::Array(vec![json!({"at": 0, "text": "t"}); n]);
        assert!(item(json!({"marks": marks(MARKS_MAX)})).is_ok());
        assert!(item(json!({"marks": marks(MARKS_MAX + 1)})).is_err());
        for bad in [
            json!({"sections": [{"name": at(41), "start": 0, "end": 1}]}),
            json!({"sections": [{"name": "S", "start": 2, "end": 1}]}),
            json!({"sections": [{"name": "S", "start": 0, "end": 1, "color": "red"}]}),
            json!({"sections": [{"start": 0, "end": 1}]}),
            json!({"marks": [{"at": 0, "text": at(301)}]}),
            json!({"marks": [{"at": 0, "text": "t", "kind": at(33)}]}),
            json!({"marks": [{"at": -1, "text": "t"}]}),
            json!({"art": "https://evil.test/a.jpg"}),
        ] {
            assert!(item(bad.clone()).is_err(), "{bad}");
        }
        for bad in [
            json!([{"title": "no id", "duration": 1}]),
            json!([{"id": "a", "duration": 1}]),
            json!([{"id": "a", "title": "A"}]),
            json!([{"id": "a", "title": "A", "duration": 1}, {"id": "a", "title": "B", "duration": 1}]),
        ] {
            assert!(set(bad.clone()).is_err(), "{bad}");
        }
        let many = |n: usize| -> Value {
            (0..n)
                .map(|n| json!({"id": n.to_string(), "title": "t", "duration": 1}))
                .collect()
        };
        assert!(set(many(QUEUE_MAX)).is_ok());
        assert!(set(many(QUEUE_MAX + 1)).is_err());
        assert_eq!(
            ask("PLAYHEAD_ITEM", json!({"id": "a", "play": true})),
            Ok(Intent::Item(Pick::Id("a".to_owned()), true))
        );
        assert_eq!(
            ask("PLAYHEAD_ITEM", json!({"index": 3})),
            Ok(Intent::Item(Pick::Index(3), false))
        );
        assert!(ask("PLAYHEAD_ITEM", json!({})).is_err());
        assert!(ask("PLAYHEAD_ITEM", json!({"index": -1})).is_err());
    }

    #[test]
    fn media_is_a_vault_relative_path_and_nothing_else() {
        for good in [
            "Media/a.ogg",
            "Recordings/Amazing Grace.m4a",
            "a",
            "Songs/Merry & Bright/01 – Joy.flac",
        ] {
            assert!(vault_path(good), "{good}");
            assert!(
                one(json!({"id": "a", "title": "A", "media": good})).is_ok(),
                "{good}"
            );
        }
        for bad in [
            "",
            "https://evil.test/a.ogg",
            "data:audio/ogg;base64,AAAA",
            "C:/a.ogg",
            "/a.ogg",
            "Media\\a.ogg",
            "../a.ogg",
            "Media/../../a.ogg",
            "Media/./a.ogg",
            "Media//a.ogg",
            "Media/",
            "Media/a.ogg?x=1",
            "Media/a.ogg#t=10",
            "%2e%2e/a.ogg",
            "Media/a%20b.ogg",
            "Media/a\u{0}.ogg",
            "Media/a\n.ogg",
            "Media/a\u{7f}.ogg",
        ] {
            assert!(!vault_path(bad), "{bad:?}");
            assert_eq!(
                one(json!({"id": "a", "title": "A", "media": bad})).unwrap_err(),
                if bad.is_empty() {
                    "an item without media needs a duration"
                } else {
                    "media must be a vault path"
                },
                "{bad:?}"
            );
        }
        assert!(!vault_path(&"a".repeat(1025)));
    }

    #[test]
    fn a_whole_queue_fits_one_message() {
        let queue: Vec<Value> = (0..QUEUE_MAX)
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
        assert!(intent(&action, &args, 0.0, "studio").is_ok());
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

    #[test]
    fn a_song_notes_timed_lyrics_and_sections() {
        let body = "Words.\n\n```lrc\n[00:00.00] [Verse 1]\n[00:01.50] Amazing grace\n[00:05.00] how sweet the sound\n[00:20.00] [Verse 2]\n[00:21.00] 'Twas grace\nnot a line\n[99:99] bad\n```\n\n```lrc\n[00:30] ignored\n```\n";
        let (lines, sections) = lyrics(body, 40.0);
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0],
            Line {
                at: 1.5,
                text: "Amazing grace".to_owned()
            }
        );
        assert_eq!(
            sections,
            [
                Section {
                    name: "Verse 1".to_owned(),
                    start: 0.0,
                    end: 20.0
                },
                Section {
                    name: "Verse 2".to_owned(),
                    start: 20.0,
                    end: 40.0
                },
            ]
        );
        assert_eq!(duration(None, &lines), 31.0);
        assert_eq!(duration(Some("2:30"), &lines), 150.0);
        assert_eq!(duration(Some("95"), &lines), 95.0);
        assert_eq!(duration(Some("x"), &[]), 180.0);
        assert!(lyrics("no fence", 10.0).0.is_empty());
    }
}
