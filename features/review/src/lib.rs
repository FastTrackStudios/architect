//! Review (`features/review/extension.toml`): Lawn-like review of a media
//! item's videos and recordings — comments pinned to a moment, a range or
//! a part of the frame, replies, @mentions, resolve and reopen, versions,
//! and open comments carried to the next version.
//!
//! One [`Review`] value runs it two ways, as every first-party extension:
//! the app links it in (`Extensions::with_linked`), and its program
//! (`src/main.rs`) serves the same value as a process, so `EXT_CLONE
//! {"id":"review"}` gives a development copy.
//!
//! - **No store of its own.** A review comment is a Threads thread on the
//!   item's note at `anchorPath` `media/<file>` with a `moment`
//!   ([`model`]); it reaches Threads through its service
//!   (`architect.threads`, `[requires]`), so comments sync as notes, show
//!   in Threads' pane and badges, and notify as Threads' do.
//! - **Versions** are the note's `review` property (else one per `video`,
//!   `media`, `recording`); `REVIEW_ADD_VERSION` writes it.
//! - **The player is the app's playhead.** The view puts the version on
//!   the playhead as Review's own source (`architect.playhead`), its
//!   comments as the item's marks, and reads the position back, so the
//!   Now Playing bar and the view move together.
//! - **Service** `architect.review`: method `m` is `REVIEW_<M>`.

#![allow(clippy::indexing_slicing)]

pub mod host;
pub mod model;
mod views;

/// The review view, for a host that draws it itself (a test, an embedder).
pub use views::ReviewView;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};

use architect_extensions::linked::{Builder, Extension};
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::model::{Moment, Note, Rect, Track};

/// Its id: its actions are `REVIEW_*`.
pub const ID: &str = "review";
/// The service it provides.
pub const SERVICE: &str = "architect.review";
/// Where its comments are kept.
pub const THREADS: &str = "architect.threads";
/// What plays.
pub const PLAYHEAD: &str = "architect.playhead";
/// Its view.
pub const VIEW: &str = "review";

/// A media item as Review shows it.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ItemInfo {
    pub path: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    /// Its `duration`, seconds, when the note gives one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    pub tracks: Vec<Track>,
}

impl ItemInfo {
    /// The track holding `file`, and that version.
    #[must_use]
    pub fn version(&self, file: &str) -> Option<(&Track, &model::Version)> {
        self.tracks
            .iter()
            .find_map(|t| t.versions.iter().find(|v| v.file == file).map(|v| (t, v)))
    }
}

/// What the view is on, as last read.
#[derive(Debug, Default)]
struct Cache {
    item: Option<ItemInfo>,
    file: Option<String>,
    /// Every version's comments, by when they start.
    notes: Vec<Note>,
    error: Option<String>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache {
    item: None,
    file: None,
    notes: Vec::new(),
    error: None,
});

/// Moves on with every read and write, for the view.
static REVISION: AtomicU64 = AtomicU64::new(0);

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE.lock().unwrap_or_else(PoisonError::into_inner)
}

fn bump() {
    REVISION.fetch_add(1, Ordering::Relaxed);
}

fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `v` without its `null` fields: an action's optional arguments left out
/// rather than sent empty.
fn no_nulls(mut v: Value) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.retain(|_, v| !v.is_null());
    }
    v
}

/// Seconds from a number or `m:ss` text.
fn seconds(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => model::parse_clock(s),
        _ => None,
    }
}

/// Media item `path`, read from its note now.
async fn load(path: &str) -> Result<ItemInfo, String> {
    let props = host::read(path).await?;
    let item = architect_media::Item::read(path, &props);
    Ok(ItemInfo {
        path: path.to_owned(),
        title: item.title,
        subtitle: item.subtitle,
        duration: item.duration,
        tracks: model::tracks(path, &props),
    })
}

/// Every review comment on item `path`, by when they start.
async fn notes(path: &str) -> Result<Vec<Note>, String> {
    let list = host::threads("list", json!({ "path": path, "resolved": true })).await?;
    Ok(model::notes_on(&list, None))
}

/// Reads what the view is on again.
async fn refresh() {
    let path = cache().item.as_ref().map(|i| i.path.clone());
    let Some(path) = path else { return };
    let (item, notes) = (load(&path).await, notes(&path).await);
    {
        let mut c = cache();
        match (item, notes) {
            (Ok(i), Ok(n)) => {
                c.item = Some(i);
                c.notes = n;
                c.error = None;
            }
            (Err(e), _) | (_, Err(e)) => c.error = Some(e),
        }
    }
    bump();
}

/// The item `args.path` names, else the note being read.
async fn item_path(args: &Value) -> Result<String, String> {
    if let Some(p) = arg(args, "path") {
        return Ok(p.to_owned());
    }
    let sel = host::selection().await;
    sel["focused_state"]
        .as_str()
        .filter(|p| {
            sel["focused_view"].as_str() == Some("vault.note")
                && std::path::Path::new(p)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("md"))
        })
        .map(str::to_owned)
        .ok_or_else(|| "give `path`: a media item's note".to_owned())
}

/// `file` of `item`, which must be one of its versions; without it its
/// first track's newest.
fn pick(item: &ItemInfo, file: Option<&str>) -> Result<String, String> {
    file.map_or_else(
        || {
            item.tracks
                .first()
                .and_then(Track::latest)
                .map(|v| v.file.clone())
                .ok_or_else(|| format!("{} names no video or recording to review", item.path))
        },
        |f| {
            item.version(f)
                .map(|(_, v)| v.file.clone())
                .ok_or_else(|| format!("{f} is not a version of {}", item.path))
        },
    )
}

/// The playhead item for `file` of `item`: its comments as marks.
fn queue_item(item: &ItemInfo, file: &str, notes: &[Note]) -> Value {
    let label = item
        .version(file)
        .map(|(t, v)| format!("{} {}", t.name, v.label))
        .unwrap_or_default();
    let marks: Vec<Value> = notes
        .iter()
        .filter(|n| n.file() == Some(file))
        .filter_map(|n| {
            let m = n.moment?;
            let text: String = n.title.chars().take(300).collect();
            Some(json!({ "at": m.start, "text": text, "kind": "comment" }))
        })
        .take(2000)
        .collect();
    let mut v = json!({
        "id": file,
        "title": item.title,
        "subtitle": label,
        "media": file,
        "marks": marks,
    });
    if let (Some(d), Some(o)) = (item.duration, v.as_object_mut()) {
        o.insert("duration".to_owned(), json!(d));
    }
    v
}

/// Puts `file` of `item` on the playhead: at `t` (where it was without),
/// playing when `play`. Without a playhead, nothing.
async fn cue(
    item: &ItemInfo,
    file: &str,
    notes: &[Note],
    t: Option<f64>,
    play: bool,
) -> Result<Value, String> {
    let mut args = json!({
        "name": "Review",
        "queue": [queue_item(item, file, notes)],
        "index": 0,
        "play": play,
    });
    if let (Some(t), Some(o)) = (t, args.as_object_mut()) {
        o.insert("time".to_owned(), json!(t));
    }
    host::playhead("set", args).await.unwrap_or(Ok(Value::Null))
}

/// `REVIEW_OPEN {path?, file?, open?}`.
async fn open(args: Value) -> Result<Value, String> {
    let path = item_path(&args).await?;
    let item = load(&path).await?;
    let file = pick(&item, arg(&args, "file"))?;
    let notes = notes(&path).await?;
    let _ = cue(&item, &file, &notes, None, false).await;
    {
        let mut c = cache();
        c.item = Some(item.clone());
        c.file = Some(file.clone());
        c.notes = notes;
        c.error = None;
    }
    bump();
    if args.get("open") != Some(&Value::Bool(false)) {
        host::run("EXT_VIEW_OPEN", json!({ "extension": ID, "view": VIEW })).await?;
    }
    Ok(json!({ "path": path, "file": file, "item": item }))
}

/// `REVIEW_ITEM {path}`.
async fn item(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?;
    let item = load(path).await?;
    let notes = notes(path).await?;
    let tracks: Vec<Value> = item
        .tracks
        .iter()
        .map(|t| {
            let versions: Vec<Value> = t
                .versions
                .iter()
                .map(|v| {
                    let on: Vec<&Note> =
                        notes.iter().filter(|n| n.file() == Some(&v.file)).collect();
                    json!({
                        "file": v.file, "label": v.label, "kind": v.kind,
                        "comments": on.len(),
                        "open": on.iter().filter(|n| !n.resolved).count(),
                    })
                })
                .collect();
            json!({ "name": t.name, "versions": versions })
        })
        .collect();
    Ok(json!({
        "path": item.path, "title": item.title, "subtitle": item.subtitle,
        "duration": item.duration, "tracks": tracks,
    }))
}

/// `REVIEW_LIST {path, file?, resolved?}`.
async fn list(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?;
    let resolved = args
        .get("resolved")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let file = arg(&args, "file");
    let shown: Vec<Note> = notes(path)
        .await?
        .into_iter()
        .filter(|n| file.is_none_or(|f| n.file() == Some(f)))
        .filter(|n| resolved || !n.resolved)
        .collect();
    Ok(json!({
        "path": path,
        "open": shown.iter().filter(|n| !n.resolved).count(),
        "comments": shown,
    }))
}

fn moment_of(args: &Value) -> Result<Moment, String> {
    let start = seconds(args.get("start")).ok_or("give `start`: seconds or m:ss")?;
    let end = match args.get("end") {
        None | Some(Value::Null) => None,
        e => Some(seconds(e).ok_or("`end`: seconds or m:ss")?),
    };
    let rect = match args.get("rect") {
        None | Some(Value::Null) => None,
        Some(r) => Some(
            serde_json::from_value::<Rect>(r.clone())
                .map_err(|_| "`rect` is {x, y, w, h}, 0 to 1".to_owned())?,
        ),
    };
    Ok(Moment { start, end, rect })
}

/// `REVIEW_COMMENT {path, file, start, end?, rect?, body, kind?, author?}`.
async fn comment(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?.to_owned();
    let item = load(&path).await?;
    let file = pick(&item, Some(arg(&args, "file").ok_or("give `file`")?))?;
    let moment = moment_of(&args)?;
    let mut start = json!({
        "path": path,
        "anchor_path": model::place(&file),
        "moment": moment,
        "body": args.get("body"),
    });
    if let Some(o) = start.as_object_mut() {
        for k in ["kind", "author", "title"] {
            if let Some(v) = args.get(k).filter(|v| !v.is_null()) {
                o.insert(k.to_owned(), v.clone());
            }
        }
    }
    let out = host::threads("start", start).await?;
    after_write(&path).await;
    Ok(out)
}

/// What a write changes: the view's comments, the playhead's marks.
async fn after_write(path: &str) {
    let on = cache().item.as_ref().is_some_and(|i| i.path == path);
    if !on {
        return;
    }
    refresh().await;
    let (item, file, notes) = {
        let c = cache();
        (c.item.clone(), c.file.clone(), c.notes.clone())
    };
    if let (Some(item), Some(file)) = (item, file) {
        // The same item: the playhead keeps its place.
        let _ = cue(&item, &file, &notes, None, false).await;
    }
}

/// `REVIEW_ADD_VERSION {path, file, track?, label?}`.
async fn add_version(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?.to_owned();
    let file = arg(&args, "file").ok_or("give `file`")?;
    let file = model::resolve(&path, file).ok_or("`file`: a vault path")?;
    let props = host::read(&path).await?;
    let tracks = model::add_version(
        model::tracks(&path, &props),
        arg(&args, "track"),
        &file,
        arg(&args, "label"),
    )?;
    host::set(&path, model::KEY, &model::review_value(&tracks)).await?;
    after_write(&path).await;
    Ok(json!({ "path": path, "file": file, "tracks": tracks }))
}

/// `REVIEW_CARRY {path, from, to?}`: every open comment of `from`, with
/// its replies, onto `to`; the old one resolved, saying where it went.
async fn carry(args: Value) -> Result<Value, String> {
    let path = arg(&args, "path").ok_or("give `path`")?.to_owned();
    let item = load(&path).await?;
    let from = pick(&item, Some(arg(&args, "from").ok_or("give `from`")?))?;
    let to = if let Some(t) = arg(&args, "to") {
        pick(&item, Some(t))?
    } else {
        {
            let (track, _) = item.version(&from).ok_or("no such version")?;
            let at = track
                .versions
                .iter()
                .position(|v| v.file == from)
                .unwrap_or(0);
            track
                .versions
                .get(at.saturating_add(1))
                .map(|v| v.file.clone())
                .ok_or_else(|| format!("{from} is its track's newest version: give `to`"))?
        }
    };
    if to == from {
        return Err("`from` and `to` are one version".to_owned());
    }
    let to_label = item
        .version(&to)
        .map(|(_, v)| v.label.clone())
        .unwrap_or_default();
    let all = notes(&path).await?;
    let mut carried = Vec::new();
    for old in model::to_carry(&all, &from) {
        let Some((first, rest)) = old.comments.split_first() else {
            continue;
        };
        let started = host::threads(
            "start",
            no_nulls(json!({
                "path": path, "anchor_path": model::place(&to), "moment": old.moment,
                "title": old.title, "kind": old.kind, "body": first.body,
                "author": first.author, "notify": false,
            })),
        )
        .await?;
        let new_id = started["thread"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let mut ids: Map<String, Value> = Map::new();
        ids.insert(first.id.clone(), started["comment"]["id"].clone());
        for c in rest {
            let reply_to = c.reply_to.as_ref().and_then(|r| ids.get(r)).cloned();
            let posted = host::threads(
                "post",
                no_nulls(json!({
                    "thread": new_id, "body": c.body, "author": c.author,
                    "reply_to": reply_to, "notify": false,
                })),
            )
            .await?;
            ids.insert(c.id.clone(), posted["comment"]["id"].clone());
        }
        host::threads(
            "post",
            json!({ "thread": old.id, "body": format!("Carried to {to_label}."), "notify": false }),
        )
        .await?;
        host::threads("resolve", json!({ "thread": old.id })).await?;
        carried.push(json!({ "from": old.id, "to": new_id }));
    }
    after_write(&path).await;
    Ok(json!({ "path": path, "from": from, "to": to, "carried": carried }))
}

/// `REVIEW_PLAY {path?, file?, t?, play?}`.
async fn play(args: Value) -> Result<Value, String> {
    let (item, notes, file) = if let Some(p) = arg(&args, "path") {
        let item = load(p).await?;
        let file = pick(&item, arg(&args, "file"))?;
        (item, notes(p).await?, file)
    } else {
        let c = cache();
        let item = c.item.clone().ok_or("give `path`")?;
        let file = if let Some(f) = arg(&args, "file") {
            pick(&item, Some(f))?
        } else {
            c.file.clone().ok_or("give `file`")?
        };
        (item, c.notes.clone(), file)
    };
    let t = args.get("t").and_then(Value::as_f64);
    let play = args.get("play").and_then(Value::as_bool).unwrap_or(true);
    cue(&item, &file, &notes, t, play).await?;
    Ok(json!({ "path": item.path, "file": file, "playing": play }))
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Review;

impl Extension for Review {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        host::connect(ext.handle());
        ext.view(VIEW, views::ReviewView)
            .action("REVIEW_OPEN", open)
            .action("REVIEW_ITEM", item)
            .action("REVIEW_LIST", list)
            .action("REVIEW_COMMENT", comment)
            .action("REVIEW_ADD_VERSION", add_version)
            .action("REVIEW_CARRY", carry)
            .action("REVIEW_PLAY", play);
    }
}
