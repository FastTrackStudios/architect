//! What the web app hears (#589): one player the app owns (never a
//! frame), an `<audio>` element or, for a recording a browser may not play
//! (Ogg Vorbis on an iPhone), the app's own decoder (`super::decoded`),
//! following the deck's active source
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
    if DEMO.contains(&path) {
        return Some("/app/demo-recording.mp3".to_owned());
    }
    // A share link's recording: the link's own URL for a file it serves
    // (`guest::file_url`), checked by the node as every request for it.
    super::guest::file_url(path).filter(|_| crate::guest_view::plays(path))
}

/// What plays the current item's media.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Via {
    /// The browser's own player: what its codecs play.
    Element,
    /// The app's decoder (`super::decoded`): Ogg Vorbis, on every browser.
    Decoded,
    /// The distribution's engine (`architect_playhead::engine`): it plays the item,
    /// and neither the element nor the decoder holds anything.
    Engine,
}

/// The app's player, as a clock: the audio element, or the app's own
/// decoder for a recording a browser may not play (`decoded::handles`).
struct AudioClock {
    el: HtmlAudioElement,
    decoded: super::decoded::Decoded,
    via: Via,
    /// Play is wanted: what a recording that turns out not to decode is
    /// started with on the element.
    wanted: bool,
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
        if self.via == Via::Engine {
            "engine"
        } else {
            "media"
        }
    }

    fn load(&mut self, media: Option<&str>) {
        // The engine first: when it carries the item it is the clock, and
        // the app's own player steps aside (one thing heard).
        let engine = architect_playhead::engine::current();
        let carried = media
            .filter(|m| architect_playhead::model::vault_path(m))
            .filter(|m| engine.as_ref().is_some_and(|e| e.borrow().carries(m)));
        if let (Some(engine), Some(media)) = (&engine, carried) {
            let key = format!("engine:{media}");
            if self.via != Via::Engine || self.src.as_deref() != Some(key.as_str()) {
                self.decoded.load(None);
                let _ = self.el.pause();
                self.el.remove_attribute("src").ok();
                self.el.load();
                self.via = Via::Engine;
                self.src = Some(key);
                self.refused.set(false);
                architect_playhead::engine::told();
                engine.borrow_mut().load(Some(media));
                self.mark();
            }
            return;
        }
        if self.via == Via::Engine {
            // The item is no longer the engine's: it stops, and the
            // element starts from nothing.
            if let Some(engine) = &engine {
                engine.borrow_mut().load(None);
            }
            self.via = Via::Element;
            self.src = None;
        }
        let url = media.and_then(media_url);
        self.refused.set(false);
        if url == self.src {
            return;
        }
        self.via = if url.as_deref().is_some_and(super::decoded::handles) {
            Via::Decoded
        } else {
            Via::Element
        };
        match (&url, self.via) {
            (Some(u), Via::Element) => {
                self.decoded.load(None);
                self.el.set_src(u);
            }
            (Some(u), Via::Decoded) => {
                let _ = self.el.pause();
                self.el.remove_attribute("src").ok();
                self.decoded.load(Some(u));
            }
            // Set just above to one of the two: the engine's items
            // returned before this.
            (Some(_), Via::Engine) => {}
            (None, _) => {
                self.decoded.load(None);
                let _ = self.el.pause();
                self.el.remove_attribute("src").ok();
            }
        }
        self.el.load();
        self.src = url;
        self.mark();
    }

    fn play(&mut self) {
        self.wanted = true;
        if self.via == Via::Engine {
            if let Some(engine) = architect_playhead::engine::current() {
                architect_playhead::engine::told();
                engine.borrow_mut().play();
            }
            return;
        }
        if self.via == Via::Decoded {
            if self.src.is_some() {
                self.decoded.play();
            }
            return;
        }
        if self.src.is_some() {
            // Refused (no gesture, no file): the silent clock goes on.
            self.refused.set(false);
            if let Ok(p) = self.el.play() {
                let _ = p.catch(&self.on_refused);
            }
        }
    }

    fn pause(&mut self) {
        self.wanted = false;
        if self.via == Via::Engine {
            if let Some(engine) = architect_playhead::engine::current() {
                architect_playhead::engine::told();
                engine.borrow_mut().pause();
            }
            return;
        }
        self.decoded.pause();
        let _ = self.el.pause();
    }

    fn seek(&mut self, t: f64) {
        if self.src.is_some() && t.is_finite() {
            match self.via {
                Via::Decoded => self.decoded.seek(t),
                Via::Element => self.el.set_current_time(t),
                Via::Engine => {
                    if let Some(engine) = architect_playhead::engine::current() {
                        architect_playhead::engine::told();
                        engine.borrow_mut().seek(t);
                    }
                }
            }
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
        if self.via == Via::Decoded {
            return Some(self.decoded.read());
        }
        if self.via == Via::Engine {
            return architect_playhead::engine::current().and_then(|e| e.borrow().read());
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

impl AudioClock {
    /// A recording the app could not decode goes to the browser's own
    /// player, from where the deck is (heard on the next sync).
    fn fall_back(&mut self) {
        if self.via != Via::Decoded || !self.decoded.failed() {
            return;
        }
        self.via = Via::Element;
        self.decoded.load(None);
        if let Some(u) = &self.src {
            self.el.set_src(u);
            self.el.load();
            if self.wanted
                && let Ok(p) = self.el.play()
            {
                let _ = p.catch(&self.on_refused);
            }
        }
        self.mark();
    }

    /// What plays, on the element, for a test to read: `data-clock`
    /// (`decoded` or `element`) and `data-media` (the URL).
    fn mark(&self) {
        let clock = match self.via {
            Via::Decoded => "decoded",
            Via::Element => "element",
            Via::Engine => "engine",
        };
        architect_playhead::engine::set_is_clock(self.via == Via::Engine);
        let _ = self.el.set_attribute("data-clock", clock);
        let _ = self
            .el
            .set_attribute("data-media", self.src.as_deref().unwrap_or(""));
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
        p.clock.fall_back();
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
        position(state.as_ref());
        if let Some(Reading { time, .. }) = p.clock.read() {
            let _ = p.clock.el.set_attribute("data-time", &format!("{time:.2}"));
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
    // The cover, where the app has a URL for the item's art (a link's own
    // file): what a lock screen and a headset's display draw.
    if let Some(src) = s["item"]["art"].as_str().and_then(art_url) {
        let image = js_sys::Object::new();
        let _ = js_sys::Reflect::set(&image, &"src".into(), &src.into());
        let _ = js_sys::Reflect::set(&init, &"artwork".into(), &js_sys::Array::of1(&image));
    }
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

/// This origin's URL for an item's art at vault path `path`: a share
/// link's own file, checked by the node as every request for it.
fn art_url(path: &str) -> Option<String> {
    architect_playhead::model::vault_path(path)
        .then(|| super::guest::file_url(path))
        .flatten()
}

/// Where the item is and how long, for the lock screen's scrubber: told
/// on every sync (a few times a second), so a seek or a pause shows at once.
fn position(state: Option<&Value>) {
    let Some(ms) = media_session() else { return };
    let Some(set) =
        get(&ms, "setPositionState").and_then(|f| f.dyn_into::<js_sys::Function>().ok())
    else {
        return;
    };
    let Some(s) = state else { return };
    let (Some(duration), Some(time)) = (s["duration"].as_f64(), s["time"].as_f64()) else {
        return;
    };
    if !(duration.is_finite() && duration > 0.0) {
        return;
    }
    let at = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&at, &"duration".into(), &duration.into());
    let _ = js_sys::Reflect::set(&at, &"position".into(), &time.clamp(0.0, duration).into());
    let _ = js_sys::Reflect::set(&at, &"playbackRate".into(), &1.0.into());
    // A browser that refuses a value throws: the scrubber is then its own.
    let _ = set.call1(&ms, &at);
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
                decoded: super::decoded::Decoded::new(sync_later),
                via: Via::Element,
                wanted: false,
                src: None,
                refused,
                on_refused,
            },
            follower: Follower::default(),
            shown: None,
        });
    });
    // The engine's answer changed (it opened, a song arrived): the item
    // is loaded again from where the deck is, by whichever now plays it.
    architect_playhead::engine::on_changed(|| {
        PLAYER.with(|p| {
            if let Ok(mut p) = p.try_borrow_mut()
                && let Some(p) = p.as_mut()
            {
                p.follower = Follower::default();
            }
        });
        sync();
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
