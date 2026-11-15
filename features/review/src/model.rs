//! Review as data: which files of a media item are reviewed, in which
//! versions, and where each comment sits on the timeline. Nothing here
//! does I/O; `crate` reads and writes through the host.
//!
//! - **Tracks and versions.** A media item (a note, `architect-media`'s
//!   `Item`) has one or more *tracks*, each a thing that is rendered
//!   again and again (the light show, the mix), and each track has
//!   *versions*, oldest first: its renders. A note says so in its
//!   `review` property ([`KEY`]); without one, each of its `video`,
//!   `media` and `recording` files is a track of one version.
//! - **Anchors.** A comment is a Threads thread on the item's note
//!   (`("note", <its path>)`), at `anchorPath` [`place`] of the version's
//!   file (`media/<the file's vault path>`), with a `moment`: a time or a
//!   range, maybe a part of the frame. So comments are per version, and
//!   Threads' own pane, counts and badges see them as comments on the
//!   note.
//! - **The timeline.** [`lanes`] stacks markers so no two overlap;
//!   [`CELLS`] equal cells take clicks and drags, so no view has to
//!   measure itself (Blitz and the browser alike).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

/// The note property that lists an item's tracks and their versions.
pub const KEY: &str = "review";

/// The properties a note without [`KEY`] is reviewed by, each a track of
/// one version, in this order.
pub const MEDIA_KEYS: [&str; 3] = ["video", "media", "recording"];

/// The first segment of a review comment's `anchorPath`.
pub const PLACE: &str = "media";

/// How many cells the timeline (and the frame, across) takes clicks in.
pub const CELLS: usize = 200;

/// How far J and L move, seconds.
pub const STEP: f64 = 5.0;

/// The `anchorPath` of comments on `file` (a vault path).
#[must_use]
pub fn place(file: &str) -> String {
    format!("{PLACE}/{}", file.trim_matches('/'))
}

/// The file an `anchorPath` is on, when it is a review's.
#[must_use]
pub fn file_of(anchor_path: Option<&str>) -> Option<&str> {
    anchor_path?
        .strip_prefix(PLACE)?
        .strip_prefix('/')
        .filter(|f| !f.is_empty())
}

/// What a file is, by its extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Video,
    Audio,
}

/// A file's [`Kind`]; `None` for one neither played nor watched.
#[must_use]
pub fn kind_of(path: &str) -> Option<Kind> {
    let ext = path.rsplit_once('.')?.1.to_ascii_lowercase();
    match ext.as_str() {
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "ogv" => Some(Kind::Video),
        "mp3" | "ogg" | "oga" | "opus" | "wav" | "flac" | "m4a" | "aac" | "aif" | "aiff" => {
            Some(Kind::Audio)
        }
        _ => None,
    }
}

/// One render of a track.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Its vault path.
    pub file: String,
    /// What it is called (`v1`, `v2`, or what the note says).
    pub label: String,
    pub kind: Kind,
}

/// A thing reviewed in versions: the light show, the mix.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub name: String,
    /// Oldest first; the last is the latest.
    pub versions: Vec<Version>,
}

impl Track {
    /// Its newest version.
    #[must_use]
    pub fn latest(&self) -> Option<&Version> {
        self.versions.last()
    }

    /// The version before `file`'s.
    #[must_use]
    pub fn before(&self, file: &str) -> Option<&Version> {
        let at = self.versions.iter().position(|v| v.file == file)?;
        self.versions.get(at.checked_sub(1)?)
    }
}

/// The folder of vault path `path` (`A/B` for `A/B/C.md`).
fn folder_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(f, _)| f)
}

/// The vault path a link in note `note`'s properties names: a path with
/// a folder as written, a bare name from the note's folder. `None` for a
/// URL, an absolute path or a `..`.
#[must_use]
pub fn resolve(note: &str, link: &str) -> Option<String> {
    let target = architect_media::link::target(link)?;
    if target.contains('/') {
        return Some(target);
    }
    Some(match folder_of(note) {
        "" => target,
        folder => format!("{folder}/{target}"),
    })
}

fn text(v: &Value) -> Option<&str> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty())
}

fn titled(key: &str) -> String {
    let mut c = key.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

fn version(note: &str, v: &Value, n: usize) -> Option<Version> {
    let (link, label) = match v {
        Value::Object(o) => (
            o.get("file").and_then(text)?,
            o.get("label").and_then(text).map(str::to_owned),
        ),
        other => (text(other)?, None),
    };
    let file = resolve(note, link)?;
    let kind = kind_of(&file)?;
    Some(Version {
        label: label.unwrap_or_else(|| format!("v{n}")),
        file,
        kind,
    })
}

/// The tracks of the item note `note` with properties `props`.
///
/// Its `review` list (`[{name, versions: ["[[file]]" | {file, label}]}]`)
/// when it has one, else each of [`MEDIA_KEYS`] that names a file a
/// track of one version. A file that is neither audio nor video, or
/// named twice, is left out.
#[must_use]
pub fn tracks(note: &str, props: &Map<String, Value>) -> Vec<Track> {
    let mut seen: Vec<String> = Vec::new();
    let mut keep = |v: Version| {
        (!seen.contains(&v.file)).then(|| {
            seen.push(v.file.clone());
            v
        })
    };
    if let Some(list) = props.get(KEY).and_then(Value::as_array) {
        return list
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let o = t.as_object()?;
                let name = o
                    .get("name")
                    .and_then(text)
                    .map_or_else(|| format!("Track {}", i.saturating_add(1)), str::to_owned);
                let versions: Vec<Version> = o
                    .get("versions")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                    .filter_map(|(n, v)| version(note, v, n.saturating_add(1)))
                    .filter_map(&mut keep)
                    .collect();
                (!versions.is_empty()).then_some(Track { name, versions })
            })
            .collect();
    }
    MEDIA_KEYS
        .iter()
        .filter_map(|k| {
            let v = version(note, props.get(*k)?, 1)?;
            let v = keep(v)?;
            Some(Track {
                name: titled(k),
                versions: vec![v],
            })
        })
        .collect()
}

/// `tracks` as the note's `review` property: each file a wikilink, so a
/// rename keeps it.
#[must_use]
pub fn review_value(tracks: &[Track]) -> Value {
    Value::Array(
        tracks
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "versions": t.versions.iter().map(|v| json!({
                        "file": format!("[[{}]]", v.file),
                        "label": v.label,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect(),
    )
}

/// `tracks` with `file` as the newest version of track `track`.
///
/// The first track when `None`; a new track of that name when none has it;
/// labelled `label` or `v<n>`. `Err` when `file` is not audio or video,
/// or is a version already.
pub fn add_version(
    mut tracks: Vec<Track>,
    track: Option<&str>,
    file: &str,
    label: Option<&str>,
) -> Result<Vec<Track>, String> {
    let kind = kind_of(file).ok_or_else(|| format!("{file} is not audio or video"))?;
    if tracks
        .iter()
        .flat_map(|t| &t.versions)
        .any(|v| v.file == file)
    {
        return Err(format!("{file} is a version already"));
    }
    let at = track.map_or_else(
        || (!tracks.is_empty()).then_some(0),
        |name| {
            tracks
                .iter()
                .position(|t| t.name.eq_ignore_ascii_case(name))
        },
    );
    let at = at.unwrap_or_else(|| {
        tracks.push(Track {
            name: track
                .unwrap_or(match kind {
                    Kind::Video => "Video",
                    Kind::Audio => "Audio",
                })
                .to_owned(),
            versions: Vec::new(),
        });
        tracks.len().saturating_sub(1)
    });
    let t = tracks
        .get_mut(at)
        .ok_or_else(|| "no such track".to_owned())?;
    let n = t.versions.len().saturating_add(1);
    t.versions.push(Version {
        file: file.to_owned(),
        label: label.map_or_else(|| format!("v{n}"), str::to_owned),
        kind,
    });
    Ok(tracks)
}

/// A part of the frame, 0 to 1 of its width and height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// When a comment is about (Threads' `moment`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Moment {
    pub start: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rect: Option<Rect>,
}

impl Moment {
    /// `m:ss`, or `m:ss–m:ss`.
    #[must_use]
    pub fn label(&self) -> String {
        self.end.map_or_else(
            || clock(self.start),
            |e| format!("{}–{}", clock(self.start), clock(e)),
        )
    }
}

/// One comment of a thread, as Threads gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub author: String,
    pub body: String,
    #[serde(default)]
    pub reply_to: Option<String>,
    #[serde(default)]
    pub posted: String,
}

/// A thread as Threads gives it (`THREADS_LIST`), the fields Review reads.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub resolved_by: Option<String>,
    #[serde(default)]
    pub anchor_path: Option<String>,
    #[serde(default)]
    pub moment: Option<Moment>,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub comments: Vec<Comment>,
}

impl Note {
    /// The file it is on, when it is a review comment.
    #[must_use]
    pub fn file(&self) -> Option<&str> {
        file_of(self.anchor_path.as_deref())
    }

    /// Where it starts, seconds (0 without a moment).
    #[must_use]
    pub fn start(&self) -> f64 {
        self.moment.map_or(0.0, |m| m.start)
    }
}

/// The review comments of a `THREADS_LIST` answer on `file` (every
/// file's when `None`), by when they start.
#[must_use]
pub fn notes_on(list: &Value, file: Option<&str>) -> Vec<Note> {
    let mut out: Vec<Note> = list
        .get("threads")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|t| serde_json::from_value::<Note>(t.clone()).ok())
        .filter(|n| n.file().is_some() && file.is_none_or(|f| n.file() == Some(f)))
        .collect();
    out.sort_by(|a, b| a.start().total_cmp(&b.start()));
    out
}

/// The unresolved comments of `from` to carry to a new version, by when
/// they start.
#[must_use]
pub fn to_carry<'a>(notes: &'a [Note], from: &str) -> Vec<&'a Note> {
    notes
        .iter()
        .filter(|n| !n.resolved && n.file() == Some(from))
        .collect()
}

/// Where along `duration` time `t` is, 0 to 100.
#[must_use]
pub fn pct(t: f64, duration: f64) -> f64 {
    if !(duration.is_finite() && duration > 0.0 && t.is_finite()) {
        return 0.0;
    }
    (t / duration * 100.0).clamp(0.0, 100.0)
}

/// The time cell `i` of [`CELLS`] starts at.
#[must_use]
pub fn cell_time(i: usize, duration: f64) -> f64 {
    let i = u32::try_from(i.min(CELLS)).unwrap_or(0);
    let cells = u32::try_from(CELLS).unwrap_or(1);
    (duration.max(0.0) * f64::from(i) / f64::from(cells)).max(0.0)
}

/// A marker's place on the timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct Marker {
    pub id: String,
    /// Its left edge and width, 0 to 100.
    pub left: f64,
    pub width: f64,
    /// Its row, from 0.
    pub lane: usize,
    pub range: bool,
    pub resolved: bool,
}

/// The narrowest a point marker is, percent.
pub const POINT_WIDTH: f64 = 1.2;

/// `notes`' markers on a timeline `duration` long, in rows so that no
/// two overlap: each goes in the first row free where it starts.
#[must_use]
pub fn lanes(notes: &[Note], duration: f64) -> Vec<Marker> {
    let mut ends: Vec<f64> = Vec::new();
    let mut sorted: Vec<&Note> = notes.iter().filter(|n| n.moment.is_some()).collect();
    sorted.sort_by(|a, b| a.start().total_cmp(&b.start()));
    sorted
        .into_iter()
        .filter_map(|n| {
            let m = n.moment?;
            let left = pct(m.start, duration);
            let right = m
                .end
                .map_or(left, |e| pct(e, duration))
                .max(left + POINT_WIDTH);
            let lane = ends.iter().position(|e| *e <= left).unwrap_or(ends.len());
            if let Some(e) = ends.get_mut(lane) {
                *e = right;
            } else {
                ends.push(right);
            }
            Some(Marker {
                id: n.id.clone(),
                left,
                width: (right - left).max(POINT_WIDTH),
                lane,
                range: m.end.is_some(),
                resolved: n.resolved,
            })
        })
        .collect()
}

/// The comment start after `now` (or before it, going back), a quarter
/// second away at least, so a press steps past the one it is on.
#[must_use]
pub fn neighbour(notes: &[Note], now: f64, forward: bool) -> Option<f64> {
    let starts = notes.iter().filter(|n| n.moment.is_some()).map(Note::start);
    if forward {
        starts.filter(|s| *s > now + 0.25).min_by(f64::total_cmp)
    } else {
        starts.filter(|s| *s < now - 0.25).max_by(f64::total_cmp)
    }
}

/// What a key does in the review view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// J, ←: back [`STEP`].
    Back,
    /// K, space: play or pause.
    Toggle,
    /// L, →: on [`STEP`].
    Forward,
    /// M: a comment at the playhead.
    Comment,
    /// `[`: the comment before.
    Previous,
    /// `]`: the comment after.
    Next,
    /// Escape: drop the comment being written, the range, the region.
    Cancel,
}

/// The key named `name` (a `KeyboardEvent`'s key), with no modifier.
#[must_use]
pub fn key(name: &str) -> Option<Key> {
    Some(match name {
        "j" | "J" | "ArrowLeft" => Key::Back,
        "k" | "K" | " " | "Spacebar" => Key::Toggle,
        "l" | "L" | "ArrowRight" => Key::Forward,
        "m" | "M" => Key::Comment,
        "[" => Key::Previous,
        "]" => Key::Next,
        "Escape" => Key::Cancel,
        _ => return None,
    })
}

/// `m:ss` (`h:mm:ss` past an hour) of `t` seconds, rounded down.
// Clamped to 0..24 h first: the cast is exact, the divisions constant.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::as_conversions,
    clippy::arithmetic_side_effects
)]
#[must_use]
pub fn clock(t: f64) -> String {
    let s = if t.is_finite() {
        t.clamp(0.0, 86_400.0).floor() as u64
    } else {
        0
    };
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Seconds from `m:ss`, `h:mm:ss`, or a number of seconds; `None`
/// otherwise.
#[must_use]
pub fn parse_clock(s: &str) -> Option<f64> {
    let mut total = 0.0_f64;
    for part in s.trim().split(':') {
        let n: f64 = part.trim().parse().ok()?;
        if !(n.is_finite() && n >= 0.0) {
            return None;
        }
        total = total.mul_add(60.0, n);
    }
    Some(total)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::float_cmp)]
mod tests {
    use super::*;

    fn props(v: &Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn a_note_without_review_has_a_track_per_media_file() {
        let t = tracks(
            "Songs/Joy/Joy.md",
            &props(&json!({
                "video": "[[Joy - Lights.mp4]]",
                "recording": "[[Recordings/Joy.ogg|mix]]",
                "media": "Joy - Lights.mp4",
                "art": "cover.png",
            })),
        );
        assert_eq!(t.len(), 2, "{t:?}");
        assert_eq!(t[0].name, "Video");
        assert_eq!(t[0].versions[0].file, "Songs/Joy/Joy - Lights.mp4");
        assert_eq!(t[0].versions[0].kind, Kind::Video);
        assert_eq!(t[1].name, "Recording");
        assert_eq!(t[1].versions[0].file, "Recordings/Joy.ogg");
        assert_eq!(t[1].versions[0].label, "v1");
        // A URL is no file.
        assert!(tracks("A.md", &props(&json!({"video": "https://x.test/a.mp4"}))).is_empty());
    }

    #[test]
    fn a_review_list_names_tracks_and_versions_and_writes_back() {
        let note = "Songs/Joy/Joy.md";
        let p = props(&json!({ "review": [
            { "name": "Lights", "versions": ["[[Joy - Lights v1.mp4]]", { "file": "[[Joy - Lights v2.mp4]]", "label": "final" }] },
            { "versions": ["Joy.ogg", "notes.pdf"] },
        ]}));
        let t = tracks(note, &p);
        assert_eq!(t[0].versions.len(), 2);
        assert_eq!(t[0].latest().unwrap().label, "final");
        assert_eq!(
            t[0].before("Songs/Joy/Joy - Lights v2.mp4").unwrap().label,
            "v1"
        );
        assert_eq!(t[1].name, "Track 2");
        assert_eq!(t[1].versions.len(), 1, "a pdf is not reviewed");
        let back = tracks(note, &props(&json!({ KEY: review_value(&t) })));
        assert_eq!(back, t);

        let t = add_version(t, Some("lights"), "Songs/Joy/Joy - Lights v3.mp4", None).unwrap();
        assert_eq!(t[0].latest().unwrap().label, "v3");
        assert!(add_version(t.clone(), None, "Songs/Joy/Joy - Lights v3.mp4", None).is_err());
        assert!(add_version(t.clone(), None, "a.txt", None).is_err());
        let t = add_version(t, Some("Stems"), "Songs/Joy/drums.wav", None).unwrap();
        assert_eq!(t[2].name, "Stems");
    }

    #[test]
    fn a_review_anchor_names_its_file() {
        assert_eq!(place("/Songs/Joy.mp4"), "media/Songs/Joy.mp4");
        assert_eq!(file_of(Some("media/Songs/Joy.mp4")), Some("Songs/Joy.mp4"));
        assert_eq!(file_of(Some("mediax/a")), None);
        assert_eq!(file_of(Some("goals/q1")), None);
        assert_eq!(file_of(None), None);
    }

    fn note(id: &str, start: f64, end: Option<f64>, resolved: bool) -> Note {
        Note {
            id: id.to_owned(),
            resolved,
            anchor_path: Some("media/a.mp4".into()),
            moment: Some(Moment {
                start,
                end,
                rect: None,
            }),
            ..Note::default()
        }
    }

    #[test]
    fn markers_never_overlap_and_points_have_a_width() {
        let n = vec![
            note("a", 10.0, Some(40.0), false),
            note("b", 20.0, None, false),
            note("c", 41.0, None, true),
            note("d", 20.5, None, false),
        ];
        let m = lanes(&n, 100.0);
        let lane = |id: &str| m.iter().find(|x| x.id == id).unwrap().lane;
        assert_eq!(lane("a"), 0);
        assert_eq!(lane("b"), 1, "inside a's range");
        assert_eq!(lane("d"), 2, "b's point is 1.2% wide");
        assert_eq!(lane("c"), 0, "after a ends");
        assert!(m.iter().all(|x| x.width >= POINT_WIDTH));
        assert_eq!(pct(150.0, 100.0), 100.0);
        assert_eq!(pct(5.0, 0.0), 0.0);
        assert_eq!(cell_time(CELLS, 200.0), 200.0);
        assert_eq!(cell_time(50, 200.0), 50.0);
    }

    #[test]
    fn carry_takes_only_open_comments_of_the_old_version() {
        let mut other = note("x", 1.0, None, false);
        other.anchor_path = Some("media/b.mp4".into());
        let n = vec![
            note("a", 5.0, None, false),
            note("b", 6.0, None, true),
            other,
        ];
        let c: Vec<&str> = to_carry(&n, "a.mp4")
            .iter()
            .map(|n| n.id.as_str())
            .collect();
        assert_eq!(c, vec!["a"]);
        assert_eq!(neighbour(&n, 5.0, true), Some(6.0));
        assert_eq!(neighbour(&n, 5.1, false), Some(1.0));
        assert_eq!(neighbour(&n, 6.0, true), None);
    }

    #[test]
    fn notes_come_from_a_threads_list_by_file_and_time() {
        let list = json!({ "threads": [
            { "id": "1", "anchorPath": "media/a.mp4", "moment": { "start": 9 }, "resolved": false, "comments": [] },
            { "id": "2", "anchorPath": "media/a.mp4", "moment": { "start": 3, "end": 4 } },
            { "id": "3", "anchorPath": "media/b.ogg", "moment": { "start": 1 } },
            { "id": "4", "anchorPath": null },
        ]});
        let ids = |f| -> Vec<String> { notes_on(&list, f).into_iter().map(|n| n.id).collect() };
        assert_eq!(ids(Some("a.mp4")), vec!["2", "1"]);
        assert_eq!(ids(None), vec!["3", "2", "1"]);
    }

    #[test]
    fn keys_and_clocks() {
        assert_eq!(key("j"), Some(Key::Back));
        assert_eq!(key(" "), Some(Key::Toggle));
        assert_eq!(key("M"), Some(Key::Comment));
        assert_eq!(key("x"), None);
        assert_eq!(clock(83.9), "1:23");
        assert_eq!(parse_clock("1:23.5"), Some(83.5));
        assert_eq!(parse_clock("1:02:03"), Some(3723.0));
        assert_eq!(parse_clock("a"), None);
        assert_eq!(
            Moment {
                start: 33.0,
                end: Some(41.0),
                rect: None
            }
            .label(),
            "0:33–0:41"
        );
    }
}
