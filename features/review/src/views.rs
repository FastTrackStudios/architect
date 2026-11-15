//! The review view (`[[views]] id = "review"`): the item's title, its
//! tracks and versions, the stage (the frame, its regions), the transport,
//! the timeline with each comment's marker (a range as a span) and the
//! comments on the version beside it, oldest moment first.
//!
//! Lawn-like and typographic: one column on a phone, the comments beside
//! the player on a desk (wrapping flex, no measuring). The timeline and
//! the frame take clicks and drags in equal cells ([`model::CELLS`]), so
//! the view never measures itself: the same in Blitz and a browser.
//!
//! The position is the playhead's (Review's own source): the view reads
//! it five times a second and seeks and toggles through it, so the Now
//! Playing bar moves with it. Without a playhead it keeps a position of
//! its own, paused.
//!
//! Keys (with the view focused, not in a field): J and ← back 5 s, K and
//! space play or pause, L and → on 5 s, M a comment at the playhead, `[`
//! and `]` the comment before and after, Escape drops what is being
//! written.

use std::sync::atomic::Ordering;
use std::time::Duration;

use dioxus::prelude::*;
use serde_json::{Value, json};

use crate::model::{self, CELLS, Comment, Key as Key2, Kind, Moment, Note, Rect, STEP};
use crate::{ItemInfo, REVISION, cache, host};

const MUTED: &str = "color: var(--muted-foreground);";
const FIELD: &str = "flex: 1; min-width: 0; padding: 7px 9px; border-radius: 6px; border: 1px solid var(--input); \
                     background: var(--background); color: var(--foreground); font-size: 13px;";
/// The frame's region grid: columns by rows.
const GRID: (usize, usize) = (16, 9);

fn button(primary: bool) -> String {
    format!(
        "padding: 5px 10px; border-radius: 6px; font-size: 12px; cursor: pointer; border: 1px solid var(--border); \
         background: {}; color: {}; white-space: nowrap;",
        if primary {
            "var(--primary)"
        } else {
            "transparent"
        },
        if primary {
            "var(--primary-foreground)"
        } else {
            "var(--foreground)"
        }
    )
}

fn chip(on: bool) -> String {
    format!(
        "padding: 3px 9px; border-radius: 999px; font-size: 12px; cursor: pointer; border: 1px solid {}; \
         background: {}; color: {}; white-space: nowrap;",
        if on {
            "var(--primary)"
        } else {
            "var(--border)"
        },
        if on { "var(--primary)" } else { "transparent" },
        if on {
            "var(--primary-foreground)"
        } else {
            "var(--foreground)"
        }
    )
}

/// Where the playhead is, as the view shows it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Position {
    time: f64,
    duration: f64,
    playing: bool,
    /// The playhead holds this version (else the view's own clock).
    ours: bool,
}

/// The playhead's state for `file`, when it holds it.
async fn read_position(file: &str) -> Option<Position> {
    let state = host::playhead("state", json!({})).await?.ok()?;
    let ours = state["item"]["id"].as_str() == Some(file);
    ours.then(|| Position {
        time: state["time"].as_f64().unwrap_or(0.0),
        duration: state["duration"].as_f64().unwrap_or(0.0),
        playing: state["playing"].as_bool().unwrap_or(false),
        ours: true,
    })
}

/// What the view is on, re-read when the cache moves; the position read
/// five times a second; the comments every ten seconds (another device's).
fn use_review(mut pos: Signal<Position>) -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        let mut ticks = 0_u32;
        loop {
            let file = cache().file.clone();
            if let Some(f) = file
                && let Some(p) = read_position(&f).await
                && *pos.peek() != p
            {
                pos.set(p);
            }
            futures_timer::Delay::new(Duration::from_millis(200)).await;
            ticks = ticks.wrapping_add(1);
            if ticks.is_multiple_of(50) {
                crate::refresh().await;
            }
            let now = REVISION.load(Ordering::Relaxed);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

/// Seeks to `t` (the playhead's, else the view's own clock).
fn seek(mut pos: Signal<Position>, t: f64) {
    let t = t.max(0.0);
    let d = pos.peek().duration;
    let t = if d > 0.0 { t.min(d) } else { t };
    pos.with_mut(|p| p.time = t);
    spawn(async move {
        let _ = host::playhead("seek", json!({ "t": t })).await;
    });
}

fn toggle(mut pos: Signal<Position>) {
    let ours = pos.peek().ours;
    spawn(async move {
        if ours {
            let _ = host::playhead("toggle", json!({})).await;
        } else {
            // Not on the playhead yet (another source took it): put it back.
            let t = pos.peek().time;
            let _ = crate::play(json!({ "t": t })).await;
        }
        if host::playhead("state", json!({})).await.is_none() {
            pos.with_mut(|p| p.playing = false);
        }
    });
}

fn run(
    work: impl std::future::Future<Output = Result<Value, String>> + 'static,
    mut problem: Signal<Option<String>>,
) {
    spawn(async move {
        match work.await {
            Ok(_) => problem.set(None),
            Err(e) => problem.set(Some(e)),
        }
    });
}

/// A Threads call from the view, then the view's comments read again.
async fn threads_then_refresh(method: &'static str, args: Value) -> Result<Value, String> {
    let out = host::threads(method, args).await?;
    crate::refresh().await;
    Ok(out)
}

/// What is being written: where, and the region.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Draft {
    start: f64,
    end: Option<f64>,
    rect: Option<Rect>,
}

/// The view.
#[component]
pub fn ReviewView() -> Element {
    let pos = use_signal(Position::default);
    let _ = use_review(pos);
    let draft = use_signal(|| None::<Draft>);
    let show_resolved = use_signal(|| false);
    let problem = use_signal(|| None::<String>);
    let (item, file, notes, error) = {
        let c = cache();
        (
            c.item.clone(),
            c.file.clone(),
            c.notes.clone(),
            c.error.clone(),
        )
    };
    let (Some(item), Some(file)) = (item, file) else {
        return rsx! {
            div { class: "review-view", style: "padding: 24px; font-size: 13px; {MUTED}",
                "Open a song or a video's note and run Review: open (or press its comments' time)."
            }
        };
    };
    let on: Vec<Note> = notes
        .iter()
        .filter(|n| n.file() == Some(file.as_str()))
        .cloned()
        .collect();
    let p = pos();
    let duration = if p.duration > 0.0 {
        p.duration
    } else {
        item.duration.unwrap_or_else(|| {
            on.iter()
                .filter_map(|n| n.moment.map(|m| m.end.unwrap_or(m.start)))
                .fold(60.0, f64::max)
        })
    };
    let kind = item.version(&file).map_or(Kind::Audio, |(_, v)| v.kind);
    let onkey = move |e: KeyboardEvent| {
        if e.modifiers().ctrl() || e.modifiers().meta() || e.modifiers().alt() {
            return;
        }
        let name = match e.key() {
            Key::Character(c) => c,
            Key::Named(n) => n.to_string(),
        };
        let Some(k) = model::key(&name) else { return };
        e.prevent_default();
        on_key(k, pos, draft);
    };
    rsx! {
        div { class: "review-view", tabindex: "0", "data-item": "{item.path}", "data-file": "{file}",
            onkeydown: onkey,
            style: "display: flex; flex-direction: column; gap: 14px; padding: 16px; height: 100%; box-sizing: border-box; \
                    overflow-y: scroll; color: var(--foreground); font-size: 13px; outline: none; \
                    font-family: var(--default-font-family, var(--font-sans));",
            Header { item: item.clone(), file: file.clone(), notes, problem }
            div { style: "display: flex; flex-wrap: wrap; gap: 16px; align-items: flex-start;",
                div { style: "flex: 3 1 420px; min-width: 0; display: flex; flex-direction: column; gap: 10px;",
                    Stage { kind, item: item.clone(), notes: on.clone(), time: p.time, draft }
                    Transport { pos, duration, draft, notes: on.clone() }
                    Timeline { notes: on.clone(), duration, pos, draft }
                    if let Some(d) = draft() {
                        Composer { item: item.path.clone(), file: file.clone(), draft: d, drafting: draft, problem }
                    }
                    if let Some(e) = problem().or(error) {
                        div { class: "review-problem", style: "color: var(--destructive); font-size: 12px;", "{e}" }
                    }
                }
                div { style: "flex: 2 1 300px; min-width: 0; display: flex; flex-direction: column; gap: 10px;",
                    CommentList { notes: on, show_resolved, pos }
                }
            }
        }
    }
}

fn on_key(k: Key2, pos: Signal<Position>, mut draft: Signal<Option<Draft>>) {
    let now = pos.peek().time;
    match k {
        Key2::Back => seek(pos, now - STEP),
        Key2::Forward => seek(pos, now + STEP),
        Key2::Toggle => toggle(pos),
        Key2::Comment => draft.set(Some(Draft {
            start: now,
            ..Draft::default()
        })),
        Key2::Cancel => draft.set(None),
        Key2::Previous | Key2::Next => {
            let notes: Vec<Note> = {
                let c = cache();
                let f = c.file.clone();
                c.notes
                    .iter()
                    .filter(|n| n.file() == f.as_deref())
                    .cloned()
                    .collect()
            };
            if let Some(t) = model::neighbour(&notes, now, k == Key2::Next) {
                seek(pos, t);
            }
        }
    }
}

// `rsx!` clones each loop row's values for its closures; clippy reads
// the last one as redundant.
#[allow(clippy::redundant_clone)]
#[component]
fn Header(
    item: ItemInfo,
    file: String,
    notes: Vec<Note>,
    problem: Signal<Option<String>>,
) -> Element {
    let open_here = notes
        .iter()
        .filter(|n| !n.resolved && n.file() == Some(file.as_str()))
        .count();
    let current_track = item.version(&file).map(|(t, _)| t.clone());
    // The version before this one, when it has open comments to carry.
    let carry = current_track.as_ref().and_then(|t| {
        let prev = t.before(&file)?;
        let n = model::to_carry(&notes, &prev.file).len();
        (n > 0).then(|| (prev.file.clone(), prev.label.clone(), n))
    });
    let path = item.path.clone();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 8px;",
            div { style: "display: flex; align-items: baseline; gap: 10px; flex-wrap: wrap;",
                span { class: "review-title", style: "font-size: 18px; font-weight: 700; letter-spacing: -0.01em;", "{item.title}" }
                if let Some(s) = &item.subtitle { span { style: "{MUTED}", "{s}" } }
                span { style: "flex: 1;" }
                span { class: "review-open", style: "font-size: 12px; {MUTED}",
                    if open_here == 1 { "1 open comment" } else { "{open_here} open comments" }
                }
            }
            if item.tracks.len() > 1 {
                div { class: "review-tracks", style: "display: flex; gap: 6px; flex-wrap: wrap;",
                    for t in item.tracks.clone() {
                        {
                            let on = current_track.as_ref().is_some_and(|c| c.name == t.name);
                            let latest = t.latest().map(|v| v.file.clone()).unwrap_or_default();
                            let p = path.clone();
                            rsx! {
                                button { key: "{t.name}", style: chip(on),
                                    onclick: move |_| run(crate::open(json!({ "path": p, "file": latest, "open": false })), problem),
                                    "{t.name}"
                                }
                            }
                        }
                    }
                }
            }
            if let Some(track) = current_track {
                div { class: "review-versions", style: "display: flex; gap: 6px; align-items: center; flex-wrap: wrap;",
                    span { style: "font-size: 12px; {MUTED}", "Version" }
                    for v in track.versions.clone() {
                        {
                            let on = v.file == file;
                            let open = notes.iter().filter(|n| !n.resolved && n.file() == Some(v.file.as_str())).count();
                            let p = path.clone();
                            let f = v.file.clone();
                            rsx! {
                                button { key: "{v.file}", class: "review-version", "data-file": "{v.file}", style: chip(on),
                                    onclick: move |_| run(crate::open(json!({ "path": p, "file": f, "open": false })), problem),
                                    "{v.label}"
                                    if open > 0 { span { style: "opacity: 0.8;", " · {open}" } }
                                }
                            }
                        }
                    }
                    if let Some((from, label, n)) = carry {
                        {
                            let p = path;
                            let to = file;
                            rsx! {
                                button { class: "review-carry", style: button(false),
                                    onclick: move |_| run(crate::carry(json!({ "path": p, "from": from, "to": to })), problem),
                                    "Carry {n} open from {label}"
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The frame (a video's) or the recording's card, the regions of the
/// comments at the playhead, and the region grid while one is drawn.
#[component]
fn Stage(
    kind: Kind,
    item: ItemInfo,
    notes: Vec<Note>,
    time: f64,
    draft: Signal<Option<Draft>>,
) -> Element {
    let mut dragging = use_signal(|| None::<(usize, usize)>);
    // A region shows from its comment's start to its end (or 3 s on).
    #[allow(clippy::needless_collect)]
    let showing: Vec<(String, Rect)> = notes
        .iter()
        .filter(|n| !n.resolved)
        .filter_map(|n| {
            let m = n.moment?;
            let r = m.rect?;
            (time >= m.start && time < m.end.unwrap_or(m.start + 3.0)).then(|| (n.id.clone(), r))
        })
        .collect();
    let drawing = draft().is_some_and(|d| d.rect.is_some() || dragging().is_some());
    let height = if kind == Kind::Video {
        "aspect-ratio: 16 / 9;"
    } else {
        "height: 120px;"
    };
    let label = model::clock(time);
    let (cols, rows) = GRID;
    let pending = draft().and_then(|d| d.rect);
    rsx! {
        div { class: "review-stage",
            style: "position: relative; width: 100%; {height} border-radius: 10px; overflow: hidden; \
                    background: #0b0b0c; color: #f4f4f5; display: flex; align-items: center; justify-content: center;",
            div { style: "display: flex; flex-direction: column; align-items: center; gap: 4px; pointer-events: none;",
                span { style: "font-size: 34px; font-weight: 600; font-variant-numeric: tabular-nums; letter-spacing: -0.02em;", "{label}" }
                span { style: "font-size: 11px; opacity: 0.6; text-transform: uppercase; letter-spacing: 0.08em;",
                    if kind == Kind::Video { "Video" } else { "Recording" }
                }
            }
            for (id, r) in showing {
                div { key: "{id}", class: "review-region",
                    style: "position: absolute; left: {r.x * 100.0}%; top: {r.y * 100.0}%; width: {r.w * 100.0}%; height: {r.h * 100.0}%; \
                            border: 2px solid #facc15; border-radius: 4px; box-sizing: border-box; pointer-events: none;",
                }
            }
            if let Some(r) = pending {
                div { style: "position: absolute; left: {r.x * 100.0}%; top: {r.y * 100.0}%; width: {r.w * 100.0}%; height: {r.h * 100.0}%; \
                              border: 2px dashed #60a5fa; background: rgba(96,165,250,0.15); box-sizing: border-box; pointer-events: none;" }
            }
            if kind == Kind::Video && draft().is_some() {
                // The region grid: press a cell, move, let go.
                div { class: "review-grid", style: "position: absolute; inset: 0; display: flex; flex-direction: column;",
                    for row in 0..rows {
                        div { key: "{row}", style: "flex: 1; display: flex;",
                            for col in 0..cols {
                                div { key: "{col}", style: "flex: 1; cursor: crosshair;",
                                    onmousedown: move |_| dragging.set(Some((col, row))),
                                    onmouseenter: move |_| if let Some(a) = dragging() { set_rect(draft, a, (col, row)) },
                                    onmouseup: move |_| {
                                        if let Some(a) = dragging() { set_rect(draft, a, (col, row)) }
                                        dragging.set(None);
                                    },
                                }
                            }
                        }
                    }
                }
            }
            if drawing {
                span { style: "position: absolute; left: 10px; bottom: 8px; font-size: 11px; opacity: 0.7;", "Drag over the frame to mark a part of it" }
            }
        }
    }
}

/// The draft's region: cells `a` to `b`, both in.
// The grid is 16 × 9: every count fits an f64 exactly.
#[allow(
    clippy::as_conversions,
    clippy::cast_precision_loss,
    clippy::arithmetic_side_effects
)]
fn set_rect(mut draft: Signal<Option<Draft>>, a: (usize, usize), b: (usize, usize)) {
    let (cols, rows) = (GRID.0 as f64, GRID.1 as f64);
    let (x0, x1) = (a.0.min(b.0) as f64, a.0.max(b.0) as f64 + 1.0);
    let (y0, y1) = (a.1.min(b.1) as f64, a.1.max(b.1) as f64 + 1.0);
    let rect = Rect {
        x: x0 / cols,
        y: y0 / rows,
        w: (x1 - x0) / cols,
        h: (y1 - y0) / rows,
    };
    draft.with_mut(|d| {
        if let Some(d) = d {
            d.rect = Some(rect);
        }
    });
}

#[component]
fn Transport(
    pos: Signal<Position>,
    duration: f64,
    draft: Signal<Option<Draft>>,
    notes: Vec<Note>,
) -> Element {
    let p = pos();
    let now = model::clock(p.time);
    let total = model::clock(duration);
    let prev_notes = notes.clone();
    rsx! {
        div { class: "review-transport", style: "display: flex; align-items: center; gap: 6px; flex-wrap: wrap;",
            button { style: button(false), title: "The comment before ([)",
                onclick: move |_| if let Some(t) = model::neighbour(&prev_notes, pos.peek().time, false) { seek(pos, t) },
                "⏮" }
            button { style: button(false), title: "Back 5 s (J)", onclick: move |_| { let t = pos.peek().time; seek(pos, t - STEP) }, "−5" }
            button { class: "review-play", style: button(true), title: "Play or pause (K, space)", onclick: move |_| toggle(pos),
                if p.playing { "Pause" } else { "Play" }
            }
            button { style: button(false), title: "On 5 s (L)", onclick: move |_| { let t = pos.peek().time; seek(pos, t + STEP) }, "+5" }
            button { style: button(false), title: "The comment after (])",
                onclick: move |_| if let Some(t) = model::neighbour(&notes, pos.peek().time, true) { seek(pos, t) },
                "⏭" }
            span { class: "review-clock", style: "font-variant-numeric: tabular-nums; margin-left: 6px;", "{now}" span { style: "{MUTED}", " / {total}" } }
            span { style: "flex: 1;" }
            button { class: "review-comment-here", style: button(false), title: "Comment at the playhead (M)",
                onclick: move |_| { let mut d = draft; d.set(Some(Draft { start: pos.peek().time, ..Draft::default() })) },
                "Comment at {now}"
            }
        }
    }
}

/// The timeline: the played part, each comment's marker in its row, and
/// [`CELLS`] cells that seek on a click and mark a range on a drag.
#[component]
fn Timeline(
    notes: Vec<Note>,
    duration: f64,
    pos: Signal<Position>,
    draft: Signal<Option<Draft>>,
) -> Element {
    let mut dragging = use_signal(|| None::<usize>);
    let mut over = use_signal(|| None::<usize>);
    let markers = model::lanes(&notes, duration);
    let count = markers.len();
    let rows = markers
        .iter()
        .map(|m| m.lane)
        .max()
        .map_or(1, |l| l.saturating_add(1));
    let height = rows.saturating_mul(10).saturating_add(26);
    let played = model::pct(pos().time, duration);
    let drag_span = dragging().zip(over()).map(|(a, b)| {
        let (a, b) = (a.min(b), a.max(b).saturating_add(1));
        (
            model::pct(model::cell_time(a, duration), duration),
            model::pct(model::cell_time(b, duration), duration),
        )
    });
    let draft_span = draft().and_then(|d| {
        d.end
            .map(|e| (model::pct(d.start, duration), model::pct(e, duration)))
    });
    rsx! {
        div { class: "review-timeline", "data-markers": "{count}",
            style: "position: relative; width: 100%; height: {height}px; border-radius: 6px; background: var(--muted); overflow: hidden;",
            div { style: "position: absolute; left: 0; top: 0; height: 4px; width: {played}%; background: var(--primary);" }
            if let Some((l, r)) = drag_span.or(draft_span) {
                div { style: "position: absolute; top: 0; bottom: 0; left: {l}%; width: {r - l}%; background: rgba(96,165,250,0.25);" }
            }
            for m in markers {
                {
                    let t = notes.iter().find(|n| n.id == m.id).map_or(0.0, Note::start);
                    let top = m.lane.saturating_mul(10).saturating_add(10);
                    let color = if m.resolved { "var(--muted-foreground)" } else { "#f59e0b" };
                    let radius = if m.range { "3px" } else { "999px" };
                    rsx! {
                        div { key: "{m.id}", class: "review-marker", "data-thread": "{m.id}", "data-lane": "{m.lane}",
                            style: "position: absolute; left: {m.left}%; width: {m.width}%; top: {top}px; height: 8px; \
                                    border-radius: {radius}; background: {color}; cursor: pointer; z-index: 2;",
                            onclick: move |e| { e.stop_propagation(); seek(pos, t) },
                        }
                    }
                }
            }
            div { class: "review-cells", style: "position: absolute; inset: 0; display: flex; z-index: 1;",
                for i in 0..CELLS {
                    div { key: "{i}", style: "flex: 1; height: 100%; cursor: pointer;",
                        onmousedown: move |_| { dragging.set(Some(i)); over.set(Some(i)); },
                        onmouseenter: move |_| if dragging().is_some() { over.set(Some(i)) },
                        onmouseup: move |_| {
                            let from = dragging().unwrap_or(i);
                            dragging.set(None);
                            over.set(None);
                            if from == i {
                                seek(pos, model::cell_time(i, duration));
                            } else {
                                let (a, b) = (from.min(i), from.max(i).saturating_add(1));
                                draft.set(Some(Draft {
                                    start: model::cell_time(a, duration),
                                    end: Some(model::cell_time(b, duration)),
                                    rect: None,
                                }));
                                seek(pos, model::cell_time(a, duration));
                            }
                        },
                    }
                }
            }
        }
    }
}

#[component]
fn Composer(
    item: String,
    file: String,
    draft: Draft,
    drafting: Signal<Option<Draft>>,
    problem: Signal<Option<String>>,
) -> Element {
    let mut body = use_signal(String::new);
    let at = Moment {
        start: draft.start,
        end: draft.end,
        rect: draft.rect,
    }
    .label();
    let send = {
        move || {
            let text = body.peek().trim().to_owned();
            if text.is_empty() {
                return;
            }
            body.set(String::new());
            let mut d = drafting;
            d.set(None);
            run(
                crate::comment(json!({
                    "path": item, "file": file, "start": draft.start, "end": draft.end,
                    "rect": draft.rect, "body": text,
                })),
                problem,
            );
        }
    };
    let mut send2 = send.clone();
    let mut send = send;
    rsx! {
        div { class: "review-composer", style: "display: flex; flex-direction: column; gap: 6px; padding: 10px; border-radius: 8px; border: 1px solid var(--primary); background: var(--card);",
            div { style: "display: flex; gap: 8px; align-items: center; font-size: 12px;",
                span { style: "font-variant-numeric: tabular-nums; font-weight: 600;", "At {at}" }
                if draft.rect.is_some() { span { style: "{MUTED}", "· a part of the frame" } }
                span { style: "flex: 1;" }
                span { style: "{MUTED}", "Esc to cancel" }
            }
            div { style: "display: flex; gap: 6px;",
                input { class: "review-body", style: FIELD, value: "{body}",
                    placeholder: "What should change here? @name to mention",
                    oninput: move |e: FormEvent| body.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        e.stop_propagation();
                        if e.key() == Key::Named(NamedKey::Enter) { send() }
                        if e.key() == Key::Named(NamedKey::Escape) { let mut d = drafting; d.set(None) }
                    },
                }
                button { class: "review-post", style: button(true), onclick: move |_| send2(), "Comment" }
                button { style: button(false), onclick: move |_| { let mut d = drafting; d.set(None) }, "Cancel" }
            }
        }
    }
}

#[component]
fn CommentList(notes: Vec<Note>, show_resolved: Signal<bool>, pos: Signal<Position>) -> Element {
    let resolved = notes.iter().filter(|n| n.resolved).count();
    let shown: Vec<Note> = notes
        .into_iter()
        .filter(|n| show_resolved() || !n.resolved)
        .collect();
    let mut sr = show_resolved;
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px;",
            span { style: "font-size: 13px; font-weight: 700;", "Comments" }
            span { style: "flex: 1;" }
            if resolved > 0 {
                button { class: "review-show-resolved", style: chip(show_resolved()), onclick: move |_| sr.toggle(), "Resolved ({resolved})" }
            }
        }
        if shown.is_empty() {
            div { style: "{MUTED} padding: 6px 0; line-height: 1.5;",
                "No comments on this version yet. Press M or “Comment at…”, or drag along the timeline for a range."
            }
        }
        for n in shown {
            NoteCard { key: "{n.id}", note: n, pos }
        }
    }
}

/// Comments with replies under what they answer, depth first.
fn ordered(comments: &[Comment]) -> Vec<(usize, Comment)> {
    fn walk(all: &[Comment], parent: Option<&str>, depth: usize, out: &mut Vec<(usize, Comment)>) {
        for c in all.iter().filter(|c| c.reply_to.as_deref() == parent) {
            out.push((depth, c.clone()));
            walk(all, Some(&c.id), depth.saturating_add(1).min(3), out);
        }
    }
    let mut out = Vec::new();
    walk(comments, None, 0, &mut out);
    for c in comments {
        if !out.iter().any(|(_, o)| o.id == c.id) {
            out.push((0, c.clone()));
        }
    }
    out
}

#[component]
fn NoteCard(note: Note, pos: Signal<Position>) -> Element {
    let mut reply = use_signal(String::new);
    let problem = use_signal(|| None::<String>);
    let id = note.id.clone();
    let resolved = note.resolved;
    let at = note.moment.map(|m| m.label()).unwrap_or_default();
    let start = note.start();
    let opacity = if resolved { "0.65" } else { "1" };
    let toggle_id = id.clone();
    let send = {
        let id = id.clone();
        move || {
            let text = reply.peek().trim().to_owned();
            if text.is_empty() {
                return;
            }
            reply.set(String::new());
            run(
                threads_then_refresh("post", json!({ "thread": id, "body": text })),
                problem,
            );
        }
    };
    let mut send2 = send.clone();
    let mut send = send;
    rsx! {
        div { class: "review-note", "data-thread": "{id}", "data-resolved": "{resolved}",
            style: "display: flex; flex-direction: column; gap: 6px; padding: 10px 12px; border-radius: 8px; \
                    border: 1px solid var(--border); background: var(--card); opacity: {opacity};",
            div { style: "display: flex; align-items: center; gap: 8px;",
                button { class: "review-at", style: "font-size: 12px; font-variant-numeric: tabular-nums; font-weight: 600; padding: 2px 7px; \
                                border-radius: 4px; border: none; background: var(--muted); color: var(--foreground); cursor: pointer;",
                    title: "Go to this moment", onclick: move |_| seek(pos, start), "{at}" }
                if note.moment.is_some_and(|m| m.rect.is_some()) { span { style: "font-size: 11px; {MUTED}", "▢ frame" } }
                span { style: "flex: 1;" }
                if resolved {
                    span { style: "font-size: 11px; {MUTED}", "Resolved" if let Some(by) = &note.resolved_by { " by {by}" } }
                }
                button { class: "review-resolve", style: button(false),
                    onclick: move |_| {
                        let m = if resolved { "reopen" } else { "resolve" };
                        run(threads_then_refresh(m, json!({ "thread": toggle_id })), problem);
                    },
                    if resolved { "Reopen" } else { "Resolve" }
                }
            }
            for (depth, c) in ordered(&note.comments) {
                div { key: "{c.id}", style: "display: flex; flex-direction: column; gap: 1px; margin-left: {depth.saturating_mul(14)}px;",
                    span { style: "font-size: 12px; font-weight: 600;", "{c.author}" }
                    div { style: "white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.45;", "{c.body}" }
                }
            }
            div { style: "display: flex; gap: 6px;",
                input { class: "review-reply", style: FIELD, value: "{reply}", placeholder: "Reply…",
                    oninput: move |e: FormEvent| reply.set(e.value()),
                    onkeydown: move |e: KeyboardEvent| {
                        e.stop_propagation();
                        if e.key() == Key::Named(NamedKey::Enter) { send() }
                    },
                }
                button { style: button(false), onclick: move |_| send2(), "Reply" }
            }
            if let Some(e) = problem() { div { style: "color: var(--destructive); font-size: 12px;", "{e}" } }
        }
    }
}
