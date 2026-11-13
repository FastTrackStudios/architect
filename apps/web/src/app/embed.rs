//! A note's extension view, chosen by the note's type (#495): an island
//! that claims the note's `type` (its listing's `embeds`, from its
//! manifest's `[[embeds]] claims = { note_type = […] }`) draws that view
//! in a frame at the head of the note.
//!
//! The island is chosen from the listing alone (`ext_host::claimed`: the
//! first island listed that claims the type, its first claim of it), and
//! only that island's bundle is fetched; a note whose type nobody claims
//! fetches nothing.
//!
//! The view is given what `ext_host::note_props` lets through of the note
//! for who is viewing (`note`; `null` unless its claim and its extension
//! both name reading it) beside the playhead's state of its extension's
//! source, flat, as `architect.playhead` v0.1 gives a frame (#589:
//! `source`, `name`, `active`, `playing`, `time`, `at`, `rate`,
//! `duration`, `index`, `tracks`, `clock`, `item`, `queue`, `title`,
//! `subtitle`, `artist`, `sections`). It may set its source
//! (`PLAYHEAD_SET`), make one item current (`PLAYHEAD_ITEM`) and use the
//! playhead's intents on it.
//!
//! The source and its clock are kept here, silent and per view, as a
//! stand-in: `architect-playhead`'s deck and the Now Playing bar (#589's
//! implementation) take their place, with the same asks and props. Nothing
//! here knows what kind of note it is: the claim chooses the view.
//!
//! A favourite's `{view}` (#607) opens any view its extension's island
//! lists the same way, in a pane of its own ([`Embedded`] with `pane`),
//! with the same props: the note, and its source's playhead state.

use std::rc::Rc;

use dioxus::prelude::*;
use serde_json::{Value, json};

use architect_shell::use_shell;

use super::frames::{Bundle, ExtensionFrame, FrameTheme, use_bundle, use_listing};
use crate::ext_host::{self, Audience, Intent, Island, Note, Pick, QueueItem};

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

/// Where the current item is: `time` seconds at `at` (ms), playing or not.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Clock {
    time: f64,
    at: f64,
    playing: bool,
}

impl Clock {
    fn time(self, duration: f64) -> f64 {
        if self.playing {
            (self.time + (now_ms() - self.at) / 1000.0).min(duration)
        } else {
            self.time
        }
    }
}

/// The theme a frame is handed: the shell's colours and its font.
fn frame_theme() -> FrameTheme {
    let t = use_shell().theme;
    let dark = architect_theme::use_resolved().scheme().is_dark();
    FrameTheme {
        vars: [
            ("--foreground", &t.ink),
            ("--muted-foreground", &t.ink_3),
            ("--background", &t.sheet),
            ("--accent", &t.accent),
            ("--accent-foreground", &t.accent_ink),
            ("--border", &t.rule),
            ("--font", &t.font),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_string()))
        .collect(),
        dark,
    }
}

/// What is played over again from its start rather than the one before,
/// once this many seconds of it have played.
const RESTART_AFTER: f64 = 3.0;

/// A view's source, as the stand-in deck keeps it: its name, its queue
/// and the item current.
#[derive(Debug, Clone, Default, PartialEq)]
struct Queue {
    name: String,
    items: Vec<QueueItem>,
    current: Option<usize>,
}

impl Queue {
    fn length(&self) -> f64 {
        self.current
            .and_then(|c| self.items.get(c))
            .and_then(|i| i.duration)
            .unwrap_or(0.0)
    }
}

/// The view that claims a note of type `note_type`, at the note's head;
/// nothing when no island claims it, or until its bundle is checked.
/// `audience` is who is viewing: what of the note the view is handed
/// depends on it (`ext_host::note_props`).
#[component]
pub fn NoteEmbed(
    audience: Audience,
    path: String,
    title: String,
    note_type: String,
    properties: Vec<(String, String)>,
    body: String,
) -> Element {
    let listing = use_listing();
    let Some(list) = listing.read().clone() else {
        return rsx! {};
    };
    let Some((island, claim)) = ext_host::claimed(&list, &note_type) else {
        return rsx! {};
    };
    let note = ext_host::note_props(
        audience,
        island,
        &Note {
            path: &path,
            title: &title,
            note_type: &note_type,
            properties: &properties,
            body: &body,
        },
    );
    rsx! {
        Claimed {
            key: "{island.folder}/{claim.view}",
            island: island.clone(),
            view: claim.view.clone(),
            note,
        }
    }
}

/// `island`'s view `view`, once its bundle is fetched and checked.
#[component]
fn Claimed(island: Island, view: String, note: Value) -> Element {
    let bundle = use_bundle(island);
    let checked: Option<Rc<Bundle>> = bundle
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok().cloned());
    let Some(bundle) = checked else {
        return rsx! {};
    };
    rsx! { Embedded { bundle, view, note } }
}

/// View `view` of `bundle`'s island on `note`, its playhead source kept
/// here: at a note's head, or filling a pane of its own (`pane`, a
/// favourite's `{view}`).
#[component]
pub fn Embedded(
    bundle: Rc<Bundle>,
    view: String,
    note: Value,
    #[props(default)] pane: bool,
) -> Element {
    let note = use_memo(use_reactive((&note,), |(n,)| n));
    let mut queue = use_signal(Queue::default);
    let mut clock = use_signal(|| Clock {
        time: 0.0,
        at: now_ms(),
        playing: false,
    });
    let mut time = use_signal(|| 0.0_f64);
    // The clock: a tick while it plays (and only then: it is started again
    // whenever the clock changes); at an item's end, the next.
    use_resource(move || async move {
        if !clock().playing {
            return;
        }
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(250)).await;
            let c = *clock.peek();
            if !c.playing {
                return;
            }
            let length = queue.peek().length();
            let now = c.time(if length > 0.0 { length } else { f64::INFINITY });
            time.set(now);
            if length > 0.0 && now >= length {
                let next = queue.peek().current.map(|n| n.saturating_add(1));
                let more = next.is_some_and(|n| n < queue.peek().items.len());
                if more {
                    queue.write().current = next;
                }
                clock.set(Clock {
                    time: if more { 0.0 } else { length },
                    at: now_ms(),
                    playing: more,
                });
                time.set(if more { 0.0 } else { length });
                return;
            }
        }
    });
    let mut start = move |current: Option<usize>, playing: bool| {
        queue.write().current = current;
        clock.set(Clock {
            time: 0.0,
            at: now_ms(),
            playing: playing && current.is_some(),
        });
        time.set(0.0);
    };
    let act = move |i: Intent| {
        let c = *clock.peek();
        let length = queue.peek().length();
        let at = c.time(if length > 0.0 { length } else { f64::INFINITY });
        let current = queue.peek().current;
        let count = queue.peek().items.len();
        match i {
            Intent::Set(source) => {
                // #589: given no index or time, the item current stays
                // current, where it was and playing or not, if it is still
                // queued; else the given index (clamped) or the first, at
                // the given time or 0. An empty queue removes the source.
                let id = current.and_then(|n| queue.peek().items.get(n).map(|i| i.id.clone()));
                let kept = id.and_then(|id| source.queue.iter().position(|i| i.id == id));
                let last = source.queue.len().checked_sub(1);
                let wanted = last.and_then(|l| source.index.map(|n| n.min(l)));
                let first = last.map(|_| 0);
                queue.write().name = source.name;
                queue.write().items = source.queue;
                match (kept, wanted, source.time) {
                    (Some(k), None, None) => {
                        queue.write().current = Some(k);
                        if source.play && !c.playing {
                            clock.set(Clock {
                                time: at,
                                at: now_ms(),
                                playing: true,
                            });
                        }
                    }
                    _ => {
                        start(wanted.or(first), source.play);
                        let t = source.time.unwrap_or(0.0);
                        let length = queue.peek().length();
                        let t = if length > 0.0 { t.min(length) } else { 0.0 };
                        let playing = clock.peek().playing;
                        clock.set(Clock {
                            time: t,
                            at: now_ms(),
                            playing,
                        });
                        time.set(t);
                    }
                }
            }
            Intent::Item(pick, play) => {
                let n = match pick {
                    Pick::Id(id) => queue.peek().items.iter().position(|i| i.id == id),
                    Pick::Index(n) => (n < count).then_some(n),
                };
                if n.is_some() {
                    start(n, play);
                }
            }
            Intent::Play if current.is_none() => start((count > 0).then_some(0), true),
            Intent::Play => clock.set(Clock {
                time: if length > 0.0 && at >= length {
                    0.0
                } else {
                    at
                },
                at: now_ms(),
                playing: true,
            }),
            Intent::Pause => {
                clock.set(Clock {
                    time: at,
                    at: now_ms(),
                    playing: false,
                });
                time.set(at);
            }
            Intent::Seek(s) => {
                clock.set(Clock {
                    time: s,
                    at: now_ms(),
                    playing: c.playing,
                });
                time.set(s);
            }
            Intent::Next => {
                if let Some(n) = current.map(|n| n.saturating_add(1)).filter(|n| *n < count) {
                    start(Some(n), c.playing);
                }
            }
            Intent::Previous => match current {
                Some(n) if at < RESTART_AFTER && n > 0 => {
                    start(Some(n.saturating_sub(1)), c.playing)
                }
                Some(_) => start(current, c.playing),
                None => {}
            },
        }
    };
    let source = bundle.island.id.clone();
    let props = use_memo(move || {
        let q = queue.read();
        let item = q.current.and_then(|c| q.items.get(c));
        // The playhead's state of this view's source, flat (#589), and the
        // note.
        json!({
            "note": note(),
            "source": source,
            "name": q.name,
            "active": item.is_some(),
            "playing": clock().playing,
            "time": time(),
            "at": js_sys::Date::now(),
            "rate": 1,
            "duration": q.length(),
            "index": q.current,
            "tracks": q.items.len(),
            "clock": "silent",
            "item": item.map(QueueItem::to_json),
            "queue": q.items.iter().map(QueueItem::to_json).collect::<Vec<_>>(),
            "title": item.map(|i| i.title.clone()),
            "subtitle": item.map(|i| i.subtitle.clone()),
            "artist": item.map(|i| i.subtitle.clone()),
            "sections": item.map_or_else(|| json!([]), QueueItem::sections_json),
        })
    });
    let theme = frame_theme();
    let duration = queue.read().length();
    let q = queue.read();
    let current = q.current.and_then(|c| q.items.get(c));
    let (testid, style) = if pane {
        (
            "ext-view-pane",
            "flex: 1 1 0; min-height: 0; display: flex; flex-direction: column; padding: 12px 16px;",
        )
    } else {
        ("note-embed", "")
    };
    rsx! {
        section { class: "web-note-embed", "data-testid": testid, "data-view": "{view}",
            "data-ext-view-of": "{view}", style,
            "data-queue": "{q.items.len()}",
            "data-current": current.map_or(String::new(), |i| i.id.clone()),
            "data-playing": "{clock().playing}",
            "data-time": "{time():.1}",
            div { style: if pane { "flex: 1 1 0; min-height: 0; border-radius: 10px; overflow: hidden;" } else { "" },
                ExtensionFrame {
                    bundle, view: view.clone(), props, duration, theme, fill: pane,
                    on_ask: act, fallback: rsx! {},
                }
            }
        }
    }
}
