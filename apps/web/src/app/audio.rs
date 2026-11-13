//! What the web app hears (#589): one `<audio>` element the app owns
//! (never a frame), following the deck's active source
//! (`architect_playhead::Follower`) and telling it back where it really
//! is, its media's length and its end (`architect_playhead::Clock`).
//!
//! An item's media is a vault path; the app turns it into this origin's
//! URL ([`media_url`]), streamed with `Range` (the page's policy allows
//! `media-src 'self'` and nothing else: `scripts/web-routes.md`). Where
//! there is none, or the browser refuses to play (its autoplay rule, a
//! missing file), the deck's own silent clock goes on and the bar still
//! moves.
//!
//! The Media Session (the lock screen, the keyboard's media keys, the
//! headset) shows the item and its source and sends play, pause, previous,
//! next and seek to the deck as the app's own presses. `navigator.
//! mediaSession` is reached through `Reflect`: web-sys has it only behind
//! its unstable flag.

// Its states are read with `value["key"]`: serde_json's index answers
// `Null` for what is missing, never a panic.
#![allow(clippy::indexing_slicing)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use architect_playhead::{Clock, Follower, Reading, now_ms};
use serde_json::{Value, json};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;
use web_sys::HtmlAudioElement;

/// The demo vault's recordings, as files the app serves itself: a silent
/// recording (`apps/web/app/demo-recording.mp3`) stands in for each.
const DEMO: &[&str] = &[
    "Recordings/Amazing Grace.mp3",
    "Recordings/Voice memo chorus melody.mp3",
    // The Christmas setlist's songs that have a recording
    // (`Setlists/Merry & Bright.md`, `apps/ui/demo-vault.py`).
    "Recordings/Christmas/Sleigh Ride.mp3",
    "Recordings/Christmas/Silent Night.mp3",
    "Recordings/Christmas/Joy to the World (Unspeakable Joy).mp3",
    "Recordings/Christmas/This Christmas.mp3",
    "Recordings/Christmas/All I Want for Christmas Is You.mp3",
];

/// This origin's URL for an item's media at vault path `path`, where the
/// app has one: the demo's recordings. A signed-in org's vault streams
/// through the node's files (`/files/{org}/{root}/content/{path}` with a
/// read ticket) once the web app opens one (#628); until then its items
/// play on the silent clock, and an item whose media is not resolved here
/// gets no length from it (`Reading::resolved`).
//
// TODO(#628): before this builds a node URL for a signed-in vault, the
// three rules of #589's media requirement (#660 M2):
// 1. resolve `media` and `art` only when the source's note (or, for a
//    queue of several notes, one of the notes the source draws from)
//    links or embeds that path; otherwise play the item on the silent
//    clock and never report a length for it;
// 2. percent-encode each path segment when building the URL (the path is
//    already `architect_playhead::model::vault_path`-checked: no `%`, `?`,
//    `#`, `..`);
// 3. the node checks the path again under the viewer's rights on the
//    stream itself (a read ticket for the viewer, never handed to a frame).
#[must_use]
pub fn media_url(path: &str) -> Option<String> {
    if !architect_playhead::model::vault_path(path) {
        return None;
    }
    DEMO.contains(&path)
        .then(|| "/app/demo-recording.mp3".to_owned())
}

/// The audio element, as a clock.
struct AudioClock {
    el: HtmlAudioElement,
    src: Option<String>,
    /// The browser refused to play it (its autoplay rule, a file it
    /// cannot play): the silent clock goes on until the next item or press.
    refused: Rc<Cell<bool>>,
    /// What a refused `play()` calls: one for the element's life, not one
    /// leaked per press.
    on_refused: Closure<dyn FnMut(JsValue)>,
}

impl Clock for AudioClock {
    fn kind(&self) -> &'static str {
        "media"
    }

    fn load(&mut self, media: Option<&str>) {
        let url = media.and_then(media_url);
        self.refused.set(false);
        if url == self.src {
            return;
        }
        if let Some(u) = &url {
            self.el.set_src(u);
        } else {
            let _ = self.el.pause();
            self.el.remove_attribute("src").ok();
        }
        self.el.load();
        self.src = url;
    }

    fn play(&mut self) {
        if self.src.is_some() {
            // Refused (no gesture, no file): the silent clock goes on.
            self.refused.set(false);
            if let Ok(p) = self.el.play() {
                let _ = p.catch(&self.on_refused);
            }
        }
    }

    fn pause(&mut self) {
        let _ = self.el.pause();
    }

    fn seek(&mut self, t: f64) {
        if self.src.is_some() && t.is_finite() {
            self.el.set_current_time(t);
        }
    }

    fn read(&self) -> Option<Reading> {
        // Media the app cannot stream here: not heard, the deck's clock.
        if self.src.is_none() {
            return Some(Reading {
                failed: true,
                ..Reading::default()
            });
        }
        let d = self.el.duration();
        Some(Reading {
            time: self.el.current_time(),
            duration: (d.is_finite() && d > 0.0).then_some(d),
            moving: !self.el.paused() && !self.el.seeking() && self.el.ready_state() >= 3,
            ended: self.el.ended(),
            failed: self.el.error().is_some() || self.refused.get(),
            // `src` is set only from `media_url`: the app resolved it.
            resolved: true,
        })
    }
}

struct Player {
    clock: AudioClock,
    follower: Follower,
    shown: Option<String>,
}

thread_local! {
    static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
}

/// Brings the element and the Media Session up to the deck: on every
/// change (inside a press, so a browser lets it play) and a few times a
/// second.
fn sync() {
    PLAYER.with(|p| {
        let Ok(mut p) = p.try_borrow_mut() else {
            return;
        };
        let Some(p) = p.as_mut() else { return };
        let now = now_ms();
        architect_playhead::with(|d| {
            p.follower.follow(d, &mut p.clock, now);
            p.follower.report(d, &p.clock, now);
        });
        let state = architect_playhead::with(|d| d.active().and_then(|a| d.state(a, now)));
        let key = state.as_ref().map(|s| {
            format!(
                "{}|{}|{}|{}",
                s["source"], s["index"], s["title"], s["playing"]
            )
        });
        if key != p.shown {
            session(state.as_ref());
            p.shown = key;
        }
    });
}

/// [`sync`] from inside a callback that may run while the player is
/// borrowed: on the next turn.
fn sync_later() {
    wasm_bindgen_futures::spawn_local(async { sync() });
}

fn get(o: &JsValue, k: &str) -> Option<JsValue> {
    js_sys::Reflect::get(o, &JsValue::from_str(k))
        .ok()
        .filter(|v| !v.is_undefined() && !v.is_null())
}

fn media_session() -> Option<JsValue> {
    let nav = web_sys::window()?.navigator();
    get(&nav, "mediaSession")
}

/// The Media Session's metadata and state for `state` (none: cleared).
fn session(state: Option<&Value>) {
    let Some(ms) = media_session() else { return };
    let Some(s) = state else {
        let _ = js_sys::Reflect::set(&ms, &"metadata".into(), &JsValue::NULL);
        let _ = js_sys::Reflect::set(&ms, &"playbackState".into(), &"none".into());
        return;
    };
    let text = |k: &str| s[k].as_str().unwrap_or_default().to_owned();
    let init = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&init, &"title".into(), &text("title").into());
    let _ = js_sys::Reflect::set(&init, &"artist".into(), &text("subtitle").into());
    let _ = js_sys::Reflect::set(&init, &"album".into(), &text("name").into());
    if let Some(ctor) = web_sys::window()
        .and_then(|w| get(&w, "MediaMetadata"))
        .and_then(|c| c.dyn_into::<js_sys::Function>().ok())
        && let Ok(meta) = js_sys::Reflect::construct(&ctor, &js_sys::Array::of1(&init))
    {
        let _ = js_sys::Reflect::set(&ms, &"metadata".into(), &meta);
    }
    let playing = if s["playing"] == json!(true) {
        "playing"
    } else {
        "paused"
    };
    let _ = js_sys::Reflect::set(&ms, &"playbackState".into(), &playing.into());
}

/// The Media Session's buttons: the app's own presses.
fn handlers() {
    let Some(ms) = media_session() else { return };
    let Some(set) =
        get(&ms, "setActionHandler").and_then(|f| f.dyn_into::<js_sys::Function>().ok())
    else {
        return;
    };
    for (name, action) in [
        ("play", "PLAYHEAD_PLAY"),
        ("pause", "PLAYHEAD_PAUSE"),
        ("previoustrack", "PLAYHEAD_PREVIOUS"),
        ("nexttrack", "PLAYHEAD_NEXT"),
        ("seekto", "PLAYHEAD_SEEK"),
    ] {
        let run = Closure::<dyn FnMut(JsValue)>::new(move |details: JsValue| {
            let args = if action == "PLAYHEAD_SEEK" {
                get(&details, "seekTime")
                    .and_then(|t| t.as_f64())
                    .map_or_else(|| json!({}), |t| json!({ "t": t }))
            } else {
                json!({})
            };
            architect_playhead::ui::press(action, &args);
        });
        // Not every browser takes every action: a refusal throws.
        let _ = set.call2(&ms, &name.into(), run.as_ref().unchecked_ref());
        run.forget();
    }
}

/// Makes the app's audio element and starts following the deck: once, at
/// the app's start. The element is the page's own, never in a frame.
pub fn start() {
    let started = PLAYER.with(|p| p.borrow().is_some());
    if started {
        return;
    }
    let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.create_element("audio").ok())
        .and_then(|e| e.dyn_into::<HtmlAudioElement>().ok())
    else {
        return;
    };
    el.set_preload("metadata");
    let _ = el.set_attribute("data-testid", "now-playing-audio");
    if let Some(body) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.body())
    {
        let _ = el.set_attribute("hidden", "");
        let _ = body.append_child(&el);
    }
    // Its own events: a loaded length or the end is heard at once.
    for ev in ["loadedmetadata", "ended", "error"] {
        let on = Closure::<dyn FnMut()>::new(sync);
        let _ = el.add_event_listener_with_callback(ev, on.as_ref().unchecked_ref());
        on.forget();
    }
    let refused = Rc::new(Cell::new(false));
    let on_refused = {
        let refused = refused.clone();
        Closure::<dyn FnMut(JsValue)>::new(move |_| {
            refused.set(true);
            // Heard at once: the bar stops saying it plays.
            sync_later();
        })
    };
    PLAYER.with(|p| {
        *p.borrow_mut() = Some(Player {
            clock: AudioClock {
                el,
                src: None,
                refused,
                on_refused,
            },
            follower: Follower::default(),
            shown: None,
        });
    });
    architect_playhead::with(|d| d.audible = true);
    // Every change, at once: inside the press that made it.
    architect_playhead::on_change("web.audio", |_| sync());
    handlers();
    wasm_bindgen_futures::spawn_local(async {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(200)).await;
            sync();
        }
    });
}
