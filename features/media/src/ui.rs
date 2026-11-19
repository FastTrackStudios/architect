//! The native list of a collection (#668).
//!
//! Drawn at a note's head in the desktop app, the phone app and the web
//! app (a share link's guest's view too): each section's name, then each row's number, cover, title over
//! subtitle, and length. A row with nothing to play is greyed. Pressing a
//! row plays it: the whole list becomes one queue on the playhead
//! (`architect.playhead`, #589), as an app-owned source, so the Now Playing
//! card shows it, next and previous follow the list, and an item's end
//! moves on to the next. The current row's title is the accent; while it
//! plays its number gives way to the equalizer mark, and paused (or not yet
//! started: a share link's guest opens with the first song current) it
//! keeps its number, in the accent, so nothing looks as if it plays.
//!
//! A caller that does something else with a row than play its media gives
//! the list its press ([`MediaList`]'s `on_press`, with `head`, `on_head`
//! and `current`): Studio's launcher loads a song's session where this
//! plays its recording. A row's [`Tile`](crate::collection::Tile) is drawn
//! where it has no cover, and over the cover's corner where it has one. A
//! list outside the shell (an extension's own process) takes its colours
//! from the theme's variables.
//!
//! After the share page's setlist (`features/sites/assets/site.css`,
//! `[data-playlist]`) and the Now Playing card: the same grid, art tile,
//! type and equalizer mark.

use architect_playhead::Caller;
use architect_shell::{FormFactor, try_use_shell};
use dioxus::prelude::*;
use lucide_dioxus::{AudioLines, ChevronRight, Pause, Play};
use serde_json::{Value, json};

use crate::collection::{Row, queue};
use crate::item::clock;

/// A list's sections: each one's name (`None`: none) and its rows.
pub type Sections = Vec<(Option<String>, Vec<Row>)>;

/// `colour` with no alpha, for a fade into it: a long title fades out
/// (Blitz has no `text-overflow: ellipsis`), and a gradient from
/// `transparent` would pass through black.
fn clear(colour: &str) -> String {
    let hex = colour.trim().trim_start_matches('#');
    let channel = |i: usize| {
        hex.get(i..i.saturating_add(2))
            .and_then(|h| u8::from_str_radix(h, 16).ok())
    };
    match (hex.len(), channel(0), channel(2), channel(4)) {
        (6 | 8, Some(r), Some(g), Some(b)) => format!("rgba({r},{g},{b},0)"),
        _ => "transparent".to_owned(),
    }
}

/// The list's sheet: classes, so its states and a phone's narrower grid
/// are rules, not each row's style. Colours come from the shell's theme
/// through the root's custom properties.
const SHEET: &str = "
.media-list { display: flex; flex-direction: column; gap: 2px; margin: 0 0 20px; font-family: var(--ml-font); color: var(--ml-ink); }
.media-list-head { display: flex; align-items: center; gap: 12px; padding: 4px 0 10px; }
.media-list-sum { flex: 1; min-width: 0; font-size: 13px; color: var(--ml-faint); font-variant-numeric: tabular-nums; }
.media-list-play { flex: none; display: flex; align-items: center; justify-content: center; gap: 6px; height: 36px; padding: 0 16px 0 12px;
  border: 0; border-radius: 999px; background: var(--ml-accent); color: var(--ml-accent-ink); font: inherit; font-size: 14px; font-weight: 600; cursor: pointer; }
.media-list-play[disabled] { opacity: .45; cursor: default; }
.media-list ol { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
.media-list-set { margin: 0; padding: 18px 0 6px; font-size: 13px; font-weight: 650; letter-spacing: .01em; color: var(--ml-faint); }
.media-list-head + .media-list-group .media-list-set { padding-top: 2px; }
.media-list-row { display: grid; grid-template-columns: minmax(0, 1fr) 44px; align-items: center; border-radius: 8px; }
.media-list-row:hover { background: var(--ml-hover); }
.media-list-main { display: grid; grid-template-columns: 1.75rem 48px minmax(0, 1fr) auto; align-items: center; column-gap: 12px;
  min-height: 60px; padding: 6px 0 6px 4px; border: 0; background: transparent; color: inherit; font: inherit; text-align: left; cursor: pointer;
  -webkit-tap-highlight-color: transparent; border-radius: 8px; }
.media-list-main:focus-visible, .media-list-open:focus-visible { outline: 2px solid var(--ml-accent); outline-offset: -2px; }
.media-list-mark { justify-self: end; font-size: 14px; font-variant-numeric: tabular-nums; color: var(--ml-faint); }
.media-list-art { position: relative; display: flex; align-items: center; justify-content: center; width: 48px; height: 48px; border-radius: 4px; overflow: hidden;
  background: var(--ml-tile); color: var(--ml-faint); }
.media-list-art img { position: absolute; top: 0; left: 0; width: 100%; height: 100%; object-fit: cover; }
.media-list-text { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.media-list-title { position: relative; overflow: hidden; white-space: nowrap; font-size: 16px; font-weight: 500; line-height: 1.3; }
.media-list-meta { position: relative; overflow: hidden; white-space: nowrap; font-size: 13.5px; line-height: 1.3; color: var(--ml-faint); }
.media-list-title::after, .media-list-meta::after { content: \"\"; position: absolute; top: 0; right: 0; bottom: 0; width: 28px;
  background: linear-gradient(to right, var(--ml-ground-clear), var(--ml-ground)); }
.media-list-length { font-size: 13px; font-variant-numeric: tabular-nums; color: var(--ml-faint); padding-right: 4px; }
.media-list-open { display: flex; align-items: center; justify-content: center; width: 44px; height: 44px; border: 0; border-radius: 999px;
  background: transparent; color: var(--ml-faint); cursor: pointer; }
.media-list-open:hover { color: var(--ml-ink); background: var(--ml-hover); }
.media-list-art[data-media-tile=\"true\"] { color: #fff; font-size: 16px; font-weight: 700; letter-spacing: -.01em; }
.media-list-art[data-media-tile=\"true\"][data-tile-size=\"small\"] { font-size: 12px; }
.media-list-key { position: absolute; right: 2px; bottom: 2px; padding: 1px 4px; border-radius: 3px; color: #fff; font-size: 10px; font-weight: 700; line-height: 1.3; }
.media-list-row[data-plays=\"false\"] .media-list-main > * { opacity: .5; }
.media-list-row[data-plays=\"false\"] .media-list-main { cursor: default; }
.media-list-row[aria-current=\"true\"] .media-list-title { color: var(--ml-accent); }
.media-list-row[data-state=\"paused\"] .media-list-mark { color: var(--ml-accent); font-weight: 600; }
.media-list-row[data-state=\"playing\"] .media-list-mark { color: transparent; position: relative; }
.media-list-row[data-state=\"playing\"] .media-list-mark::after { content: \"\"; position: absolute; right: 0; top: 50%; width: 13px; height: 13px; margin-top: -6.5px;
  background: linear-gradient(var(--ml-accent), var(--ml-accent)) 0 100% / 3px 60% no-repeat,
    linear-gradient(var(--ml-accent), var(--ml-accent)) 5px 100% / 3px 100% no-repeat,
    linear-gradient(var(--ml-accent), var(--ml-accent)) 10px 100% / 3px 40% no-repeat; }
.media-list-row[data-state=\"playing\"] .media-list-mark::after { animation: media-list-eq .9s ease-in-out infinite alternate; }
@keyframes media-list-eq { 0% { background-size: 3px 60%, 3px 100%, 3px 40%; } 50% { background-size: 3px 100%, 3px 45%, 3px 80%; } 100% { background-size: 3px 35%, 3px 75%, 3px 100%; } }
@media (prefers-reduced-motion: reduce) { .media-list-row[data-state=\"playing\"] .media-list-mark::after { animation: none; } }
.media-list[data-phone=\"true\"] .media-list-main { grid-template-columns: 22px 48px minmax(0, 1fr) auto; column-gap: 10px; padding-left: 0; }
.media-list[data-phone=\"true\"] .media-list-mark { font-size: 13px; letter-spacing: -.02em; }
.media-list[data-phone=\"true\"] .media-list-length { display: none; }
";

/// Whether source `source`'s queue is `queue` already (same ids, same
/// order).
fn holds(source: &str, queue: &[Value]) -> bool {
    architect_playhead::with(|d| {
        d.source(source).is_some_and(|s| {
            s.queue.len() == queue.len()
                && s.queue
                    .iter()
                    .zip(queue)
                    .all(|(i, q)| q["id"].as_str() == Some(i.id.as_str()))
        })
    })
}

/// Play row `id` of the list: the list becomes source `source`'s queue
/// (when it is not already) with that row current, playing.
fn play(source: &str, name: &str, queue: &[Value], id: &str) {
    let asked = if holds(source, queue) {
        architect_playhead::ask(
            &Caller::App,
            "PLAYHEAD_ITEM",
            &json!({ "source": source, "id": id, "play": true }),
        )
    } else {
        let index = queue.iter().position(|q| q["id"].as_str() == Some(id));
        architect_playhead::ask(
            &Caller::App,
            "PLAYHEAD_SET",
            &json!({ "source": source, "name": name, "queue": queue, "index": index.unwrap_or(0), "play": true }),
        )
    };
    if let Err(why) = asked {
        tracing::warn!(source, id, error = %why, "media.list.play_refused");
    }
}

/// Pause or go on with source `source`.
fn toggle(source: &str) {
    let _ = architect_playhead::ask(
        &Caller::App,
        "PLAYHEAD_TOGGLE",
        &json!({ "source": source }),
    );
}

/// The list's colours: the shell's theme, or the theme's variables where
/// no shell is around it.
#[derive(Clone, PartialEq)]
struct Look {
    ink: String,
    faint: String,
    accent: String,
    accent_ink: String,
    hover: String,
    tile: String,
    font: String,
    ground: String,
    ground_clear: String,
}

impl Look {
    /// The shell's, when the list is in one.
    fn of(shell: Option<&architect_shell::Shell>) -> Self {
        shell.map_or_else(
            || Self {
                ink: "var(--foreground, #e6e8eb)".to_owned(),
                faint: "var(--muted-foreground, #8b9099)".to_owned(),
                accent: "var(--signal-safe, #3ecf8e)".to_owned(),
                accent_ink: "#06140d".to_owned(),
                hover: "var(--accent, #26292f)".to_owned(),
                tile: "var(--muted, #26292f)".to_owned(),
                font: "inherit".to_owned(),
                ground: "var(--background, #101114)".to_owned(),
                ground_clear: "transparent".to_owned(),
            },
            |shell| {
                let t = &shell.theme;
                Self {
                    ink: t.ink.to_string(),
                    faint: t.ink_3.to_string(),
                    accent: t.accent.to_string(),
                    accent_ink: t.accent_ink.to_string(),
                    hover: t.active.to_string(),
                    tile: t.pill.to_string(),
                    font: t.font.to_string(),
                    ground: t.sheet.to_string(),
                    ground_clear: clear(&t.sheet),
                }
            },
        )
    }
}

/// A collection's list. `source` is the app's own playhead source it
/// plays as (outside the extension-id grammar: `app:…`), `name` what the
/// Now Playing card names it. `art` turns a row's art (a vault path) into
/// an image's URL where the app can; `on_open` opens a row's note (its
/// path), offered as each row's chevron and as a greyed row's press.
///
/// With `on_press`, a row is pressed rather than played: it is told the
/// row's id, nothing goes to the playhead, and the row that is current is
/// `current` (a row's id). The pill then says `head` and runs `on_head`,
/// and `sum` replaces the line beside it. `phone` is the narrow grid
/// where no shell says what the screen is.
#[component]
pub fn MediaList(
    source: String,
    name: String,
    sections: Sections,
    #[props(default)] art: Option<Callback<String, Option<String>>>,
    #[props(default)] on_open: Option<EventHandler<String>>,
    #[props(default)] on_press: Option<EventHandler<String>>,
    #[props(default)] on_head: Option<EventHandler<()>>,
    #[props(default)] head: Option<String>,
    #[props(default)] sum: Option<String>,
    #[props(default)] current: Option<String>,
    #[props(default)] phone: bool,
) -> Element {
    let shell = try_use_shell();
    let t = Look::of(shell.as_ref());
    let phone = shell.map_or(phone, |s| s.screen.form() == FormFactor::Phone);
    let pressed = on_press.is_some();
    let now = architect_playhead::ui::use_playhead();
    let q = queue(&sections);
    let total: f64 = sections
        .iter()
        .flat_map(|(_, r)| r.iter())
        .filter_map(|r| r.item.duration)
        .sum();
    let count: usize = sections.iter().map(|(_, r)| r.len()).sum();
    let playable = q.len();
    let (current, playing) = if pressed {
        // The caller's own: what it loaded, never "playing".
        (current, false)
    } else {
        now().active.as_ref().map_or((None, false), |a| {
            if a.source == source {
                (a.item.as_ref().map(|i| i.id.clone()), a.playing)
            } else {
                (None, false)
            }
        })
    };
    let ours_playing = current.is_some() && playing;
    let sum = sum.unwrap_or_else(|| {
        let mut s = if count == 1 {
            "1 item".to_owned()
        } else {
            format!("{count} items")
        };
        if total > 0.0 {
            s.push_str(" · ");
            s.push_str(&clock(total));
        }
        if playable < count {
            s.push_str(" · ");
            s.push_str(&playable.to_string());
            s.push_str(" to play");
        }
        s
    });
    let vars = format!(
        "--ml-ink: {}; --ml-faint: {}; --ml-accent: {}; --ml-accent-ink: {}; --ml-hover: {}; --ml-tile: {}; --ml-font: {}; \
         --ml-ground: {}; --ml-ground-clear: {};",
        t.ink, t.faint, t.accent, t.accent_ink, t.hover, t.tile, t.font, t.ground, t.ground_clear
    );
    let head_play = {
        let (source, name, q) = (source.clone(), name.clone(), q.clone());
        let current = current.clone();
        move |_| {
            if let Some(head) = on_head {
                head.call(());
            } else if current.is_some() {
                toggle(&source);
            } else if let Some(first) = q.first().and_then(|i| i["id"].as_str()) {
                play(&source, &name, &q, first);
            }
        }
    };
    rsx! {
        style { {SHEET} }
        section {
            class: "media-list",
            "data-testid": "media-list",
            "data-source": "{source}",
            "data-phone": "{phone}",
            "aria-label": "{name}",
            style: "{vars}",
            div { class: "media-list-head",
                button {
                    class: "media-list-play",
                    "data-testid": "media-list-play",
                    disabled: playable == 0,
                    "aria-label": if ours_playing { "Pause".to_owned() } else { head.unwrap_or_else(|| "Play".to_owned()) },
                    onclick: head_play,
                    if ours_playing {
                        Pause { size: 18, color: t.accent_ink.clone() }
                        "Pause"
                    } else {
                        Play { size: 18, color: t.accent_ink.clone() }
                        {head.clone().unwrap_or_else(|| "Play".to_owned())}
                    }
                }
                span { class: "media-list-sum", "data-testid": "media-list-sum", "{sum}" }
            }
            for (s, (label, rows)) in sections.iter().enumerate() {
                div { key: "{s}", class: "media-list-group",
                if let Some(label) = label.clone() {
                    h3 { class: "media-list-set", "data-media-set": "{label}", "{label}" }
                }
                ol {
                    for row in rows.iter().cloned() {
                        ListRow {
                            key: "{row.id}",
                            row: row.clone(),
                            current: current.as_deref() == Some(row.id.as_str()),
                            playing,
                            picture: row.art.clone().and_then(|a| art.and_then(|f| f.call(a))),
                            faint: t.faint.clone(),
                            pressed,
                            on_play: {
                                let (source, name, q) = (source.clone(), name.clone(), q.clone());
                                move |id: String| match on_press {
                                    Some(press) => press.call(id),
                                    None => play(&source, &name, &q, &id),
                                }
                            },
                            on_toggle: {
                                let (source, id) = (source.clone(), row.id.clone());
                                move |()| match on_press {
                                    Some(press) => press.call(id.clone()),
                                    None => toggle(&source),
                                }
                            },
                            on_open,
                        }
                    }
                }
                }
            }
        }
    }
}

/// One row.
#[component]
fn ListRow(
    row: Row,
    current: bool,
    playing: bool,
    picture: Option<String>,
    faint: String,
    pressed: bool,
    on_play: EventHandler<String>,
    on_toggle: EventHandler<()>,
    on_open: Option<EventHandler<String>>,
) -> Element {
    let plays = row.plays();
    let path = row.item.path.clone();
    let can_open = on_open.is_some() && !path.is_empty();
    let state = if current && playing {
        "playing"
    } else if current {
        "paused"
    } else {
        "idle"
    };
    let id = row.id.clone();
    let press = {
        let path = path.clone();
        move |_| {
            if !plays {
                if let (Some(open), false) = (on_open, path.is_empty()) {
                    open.call(path.clone());
                }
            } else if current {
                on_toggle.call(());
            } else {
                on_play.call(id.clone());
            }
        }
    };
    let has_art = picture.is_some();
    let label = match (plays, current && playing, pressed) {
        (true, _, true) => format!("Open {}", row.item.title),
        (true, true, false) => format!("Pause {}", row.item.title),
        (true, false, false) => format!("Play {}", row.item.title),
        (false, _, true) => format!("{}: nothing to open", row.item.title),
        (false, _, false) => format!("{}: nothing to play", row.item.title),
    };
    // Its tile: the whole square where it has no cover, the corner where
    // it has one.
    let tile = row.tile.clone();
    let tile_fills = tile.is_some() && !has_art;
    let tile_ground = tile
        .as_ref()
        .map(|t| format!("background: hsl({} 42% 40%);", t.hue))
        .unwrap_or_default();
    let tile_size = if tile.as_ref().is_some_and(|t| t.text.chars().count() > 2) {
        "small"
    } else {
        "normal"
    };
    rsx! {
        li {
            class: "media-list-row",
            "data-media-row": "{row.item.title}",
            "data-plays": "{plays}",
            "data-state": "{state}",
            "aria-current": "{current}",
            button {
                class: "media-list-main",
                "aria-label": "{label}",
                "aria-disabled": "{!plays && !can_open}",
                onclick: press,
                span { class: "media-list-mark", "{row.number}" }
                span {
                    class: "media-list-art",
                    "data-media-art": "{has_art}",
                    "data-media-tile": "{tile_fills}",
                    "data-tile-size": "{tile_size}",
                    style: if tile_fills { tile_ground } else { String::new() },
                    if let Some(src) = picture {
                        img { src: "{src}", alt: "", draggable: "false" }
                        if let Some(tile) = tile {
                            span { class: "media-list-key", "data-media-key": "{tile.text}", style: "{tile_ground}", "{tile.text}" }
                        }
                    } else if let Some(tile) = tile {
                        "{tile.text}"
                    } else {
                        AudioLines { size: 22, color: faint.clone() }
                    }
                }
                span { class: "media-list-text",
                    span { class: "media-list-title", "{row.item.title}" }
                    span { class: "media-list-meta",
                        if let Some(sub) = row.item.subtitle.clone() {
                            "{sub}"
                        } else if !plays && !pressed {
                            "Nothing to play"
                        }
                    }
                }
                span { class: "media-list-length",
                    if let Some(d) = row.item.duration { {clock(d)} }
                }
            }
            if can_open {
                button {
                    class: "media-list-open",
                    "data-media-open": "{row.item.title}",
                    "aria-label": "Open {row.item.title}",
                    onclick: move |_| {
                        if let Some(open) = on_open {
                            open.call(path.clone());
                        }
                    },
                    ChevronRight { size: 20, color: faint }
                }
            } else {
                span {}
            }
        }
    }
}
