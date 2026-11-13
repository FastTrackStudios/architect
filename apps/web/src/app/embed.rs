//! A note's extension view, chosen by the note's type (#495): an island
//! that claims the note's `type` (its listing's `embeds`, from its
//! manifest's `[[embeds]] claims = { note_type = […] }`) draws that view
//! in a frame at the head of the note.
//!
//! The view is given the note as data (`note`: `{path, title, type,
//! properties, body}`) beside the playhead's state of its extension's
//! source, flat, as `architect.playhead` v0.1 gives a frame (#589:
//! `source`, `name`, `playing`, `time`, `at`, `rate`, `duration`, `index`,
//! `tracks`, `clock`, `item`, `queue`, `title`, `subtitle`, `artist`,
//! `sections`). It may set its source (`PLAYHEAD_SET`), make one item
//! current (`PLAYHEAD_ITEM`) and use the playhead's intents on it.
//!
//! The source and its clock are kept here, silent and per view, as a
//! stand-in: `architect-playhead`'s deck and the Now Playing bar (#589's
//! implementation) take their place, with the same asks and props. Nothing
//! here knows what kind of note it is: the claim chooses the view.

use std::rc::Rc;

use dioxus::prelude::*;
use serde_json::{Map, Value, json};

use super::frames::{Bundle, ExtensionFrame, use_island};
use super::song::{Clock, frame_theme, now_ms};
use crate::ext_host::{Intent, QueueItem};

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

fn item_json(i: &QueueItem) -> Value {
    json!({"id": i.id, "title": i.title, "subtitle": i.subtitle, "duration": i.duration, "media": i.media})
}

/// The view that claims a note of type `note_type`, at the note's head;
/// nothing when no island claims it, or until its bundle is checked.
#[component]
pub fn NoteEmbed(
    path: String,
    title: String,
    note_type: String,
    properties: Vec<(String, String)>,
    body: String,
) -> Element {
    let island = use_island();
    let bundle: Option<Rc<Bundle>> = island
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok().cloned());
    let Some(bundle) = bundle else {
        return rsx! {};
    };
    let Some(view) = bundle
        .island
        .embed_for(note_type.as_str())
        .map(str::to_owned)
    else {
        return rsx! {};
    };
    let mut props = Map::new();
    for (k, v) in properties {
        props.insert(k, Value::String(v));
    }
    let note = json!({
        "path": path, "title": title, "type": note_type,
        "properties": props, "body": body,
    });
    rsx! { Embedded { bundle, view, note } }
}

#[component]
fn Embedded(bundle: Rc<Bundle>, view: String, note: Value) -> Element {
    let note = use_memo(use_reactive((&note,), |(n,)| n));
    let mut queue = use_signal(Queue::default);
    let mut clock = use_signal(|| Clock {
        time: 0.0,
        at: now_ms(),
        playing: false,
    });
    let mut time = use_signal(|| 0.0_f64);
    // The clock: a tick while it plays; at a song's end, the next.
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(250)).await;
            let c = *clock.peek();
            if !c.playing {
                continue;
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
            Intent::Set(name, items) => {
                // #589: the item current stays current, where it was and
                // playing or not, if it is still queued; else the first,
                // paused at 0. An empty queue removes the source.
                let id = current.and_then(|n| queue.peek().items.get(n).map(|i| i.id.clone()));
                let kept = id.and_then(|id| items.iter().position(|i| i.id == id));
                let first = (!items.is_empty()).then_some(0);
                queue.write().name = name;
                queue.write().items = items;
                if kept.is_some() {
                    queue.write().current = kept;
                } else {
                    start(first, false);
                }
            }
            Intent::Item(id, play) => {
                let n = queue.peek().items.iter().position(|i| i.id == id);
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
            "item": item.map(item_json),
            "queue": q.items.iter().map(item_json).collect::<Vec<_>>(),
            "title": item.map(|i| i.title.clone()),
            "subtitle": item.map(|i| i.subtitle.clone()),
            "artist": item.map(|i| i.subtitle.clone()),
            "sections": [],
        })
    });
    let theme = frame_theme();
    let duration = queue.read().length();
    let q = queue.read();
    let current = q.current.and_then(|c| q.items.get(c));
    rsx! {
        section { class: "web-note-embed", "data-testid": "note-embed", "data-view": "{view}",
            "data-queue": "{q.items.len()}",
            "data-current": current.map_or(String::new(), |i| i.id.clone()),
            "data-playing": "{clock().playing}",
            ExtensionFrame {
                bundle, view: view.clone(), props, duration, theme,
                on_ask: act, fallback: rsx! {},
            }
        }
    }
}
