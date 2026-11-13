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
/// dropped unread.
pub const MAX_MESSAGE: usize = 16 * 1024;

/// What a frame may ask the app: the playhead's intents, nothing else
/// (what an island's `capabilities.actions` holds; host calls into a
/// vault are #495's next step, authorised in the app and the node).
pub const INTENTS: &[&str] = &[
    "PLAYHEAD_SEEK",
    "PLAYHEAD_PLAY",
    "PLAYHEAD_PAUSE",
    "PLAYHEAD_NEXT",
    "PLAYHEAD_PREVIOUS",
];

/// One island's web bundle, as the listing names it and this host
/// believes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Island {
    /// Its extension's id (`studio.transport`).
    pub id: String,
    /// Its folder on the node, `/_ext/island/<id>.<hash>/`.
    pub folder: String,
    /// The web view document its frames load (`/_ext/webview.<hash>.html`,
    /// or `webview-rtc` when its WebRTC is declared and granted).
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

/// Whether `path` is one of the node's web view documents.
#[must_use]
pub fn webview_ok(path: &str) -> bool {
    hex_after(path, "/_ext/webview.", ".html").is_some()
        || hex_after(path, "/_ext/webview-rtc.", ".html").is_some()
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
    })
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
}

/// `action` with `args` as an [`Intent`], if it is one and its arguments
/// are what it takes (a seek within `0..=duration`).
#[must_use]
pub fn intent(action: &str, args: &Value, duration: f64) -> Option<Intent> {
    match action {
        "PLAYHEAD_SEEK" => {
            let t = args.get("t").and_then(Value::as_f64)?;
            (t.is_finite() && t >= 0.0 && duration > 0.0 && t <= duration)
                .then_some(Intent::Seek(t))
        }
        "PLAYHEAD_PLAY" => Some(Intent::Play),
        "PLAYHEAD_PAUSE" => Some(Intent::Pause),
        "PLAYHEAD_NEXT" => Some(Intent::Next),
        "PLAYHEAD_PREVIOUS" => Some(Intent::Previous),
        _ => None,
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
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
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
    fn asks_are_the_playheads_intents_and_rate_limited() {
        let mut b = Bucket::new(0.0);
        let burst = (0..100).filter(|_| b.take(0.0)).count();
        assert_eq!(burst, 16);
        assert!(!b.take(10.0));
        // A second later: eight more.
        let more = (0..100).filter(|_| b.take(1000.0)).count();
        assert_eq!(more, 8);
        assert_eq!(
            intent("PLAYHEAD_SEEK", &json!({"t": 3.5}), 10.0),
            Some(Intent::Seek(3.5))
        );
        assert_eq!(intent("PLAYHEAD_SEEK", &json!({"t": 30}), 10.0), None);
        assert_eq!(intent("PLAYHEAD_SEEK", &json!({"t": -1}), 10.0), None);
        assert_eq!(intent("PLAYHEAD_SEEK", &json!({}), 10.0), None);
        assert_eq!(
            intent("PLAYHEAD_PLAY", &Value::Null, 10.0),
            Some(Intent::Play)
        );
        assert_eq!(intent("VAULT_READ", &json!({"path": "a"}), 10.0), None);
        assert_eq!(intent("SHARE_CREATE", &Value::Null, 10.0), None);
        assert!(
            INTENTS
                .iter()
                .all(|a| intent(a, &json!({"t": 1}), 10.0).is_some())
        );
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
