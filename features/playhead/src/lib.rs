//! The playhead (`features/playhead/extension.toml`, #589): the app's one
//! media service and the Now Playing bar it feeds.
//!
//! Any extension puts what it plays on the deck as its **source** (a
//! queue of items: title, subtitle, art, length, sections, timed marks,
//! media by vault path) and controls it; several extensions can each have
//! one, and the bar shows the **active** one. Nothing here knows what a
//! source means to its extension.
//!
//! - [`model`]: a source's items and the rules each meets.
//! - [`deck`]: the sources, the active one, who may move which.
//! - [`clock`]: the seam to what is heard ([`clock::Clock`]): the web
//!   app's audio element, a native app's silent clock, later an engine.
//! - [`ask`]: one door for every runtime. A linked or process extension's
//!   service call (`architect.playhead`, method `m` is `PLAYHEAD_<M>`) and
//!   a web bundle's ask in the web app's frame host both come here, as
//!   their extension (the host names it, never the arguments).
//! - `ui` (feature `ui`): `ui::use_playhead` and the bar,
//!   `ui::NowPlayingBar`, in the shell's `now_playing` slot.
//! - `extension` (feature `extension`): `Playhead`, its
//!   `linked::Extension`, which the app links in.
//!
//! One deck per running app ([`with`]); the playhead is not synced state:
//! a position is one device's, moves ten times a second and has nothing
//! to merge. What a source plays comes from its extension's documents,
//! which sync through `crdt::docs`.

pub mod clock;
pub mod deck;
pub mod model;
#[cfg(feature = "ui")]
pub mod ui;

#[cfg(feature = "extension")]
mod extension;

use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};

pub use crate::clock::{Clock, Follower, Reading, Silent};
pub use crate::deck::{Caller, Control, Deck, Pick, Source};
#[cfg(feature = "extension")]
pub use crate::extension::Playhead;
pub use crate::model::{Item, Mark, Section, Spec};

/// Its id: its actions are `PLAYHEAD_*`.
pub const ID: &str = "playhead";

/// The service it provides: method `m` is `PLAYHEAD_<M>`.
pub const SERVICE: &str = "architect.playhead";

/// The app event raised on each change, its detail the source's state.
pub const CHANGED: &str = "playhead.changed";

/// Every action an extension may run on it (the frame host lets a web
/// bundle ask the ones in [`FRAME_ASKS`]).
pub const ACTIONS: &[&str] = &[
    "PLAYHEAD_SET",
    "PLAYHEAD_PLAY",
    "PLAYHEAD_PAUSE",
    "PLAYHEAD_TOGGLE",
    "PLAYHEAD_SEEK",
    "PLAYHEAD_NEXT",
    "PLAYHEAD_PREVIOUS",
    "PLAYHEAD_ITEM",
    "PLAYHEAD_STATE",
    "PLAYHEAD_SOURCES",
    "PLAYHEAD_ACTIVATE",
];

/// What a web bundle in a frame may ask: the five #656 forwarded, and
/// `ITEM` and `SET` (its props are its state, so no reads).
pub const FRAME_ASKS: &[&str] = &[
    "PLAYHEAD_SEEK",
    "PLAYHEAD_PLAY",
    "PLAYHEAD_PAUSE",
    "PLAYHEAD_NEXT",
    "PLAYHEAD_PREVIOUS",
    "PLAYHEAD_ITEM",
    "PLAYHEAD_SET",
];

/// The runtime state an app mirrors each source's playing into, as its
/// extension's own (`ext:<id>`, this key, `true` or `false`): what a
/// favourite's primary (`state = "playing"`, #607) shows its Pause face by.
pub const PLAYING_STATE: &str = "playing";

/// Whether source `id` (an extension's id) is playing now.
#[must_use]
pub fn playing(id: &str) -> bool {
    with(|d| d.source(id).is_some_and(|s| s.transport.playing))
}

/// Now, ms since the epoch (the `at` of every state).
#[must_use]
pub fn now_ms() -> f64 {
    web_time::SystemTime::now()
        .duration_since(web_time::UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

static DECK: Mutex<Option<Deck>> = Mutex::new(None);

type Listener = Arc<dyn Fn(&Value) + Send + Sync>;
static LISTENER: Mutex<Option<Listener>> = Mutex::new(None);

/// Works on the app's one deck.
pub fn with<R>(f: impl FnOnce(&mut Deck) -> R) -> R {
    let mut d = DECK.lock().unwrap_or_else(PoisonError::into_inner);
    f(d.get_or_insert_with(Deck::new))
}

/// The deck's revision: moves on with every change.
#[must_use]
pub fn revision() -> u64 {
    with(|d| d.revision())
}

/// Called with a source's state on each change (the app raises
/// [`CHANGED`] from it); replaces the one before.
pub fn on_change(listener: impl Fn(&Value) + Send + Sync + 'static) {
    *LISTENER.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(listener));
}

fn changed(state: &Value) {
    let l = LISTENER
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(l) = l {
        l(state);
    }
}

fn source_arg(args: &Value) -> Option<String> {
    args.get("source")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// `action` with `args` as a [`Control`], and the source it names.
///
/// # Errors
///
/// Not one of its controls, or its arguments are not what it takes.
pub fn control(action: &str, args: &Value) -> Result<(Control, Option<String>), String> {
    let c = match action {
        "PLAYHEAD_SET" => Control::Set(Spec::parse(args)?),
        "PLAYHEAD_PLAY" => Control::Play,
        "PLAYHEAD_PAUSE" => Control::Pause,
        "PLAYHEAD_NEXT" => Control::Next,
        "PLAYHEAD_PREVIOUS" => Control::Previous,
        "PLAYHEAD_ACTIVATE" => Control::Activate,
        "PLAYHEAD_SEEK" => Control::Seek(
            args.get("t")
                .and_then(Value::as_f64)
                .ok_or("give `t`, seconds")?,
        ),
        "PLAYHEAD_ITEM" => {
            let pick = match (args.get("index").and_then(Value::as_u64), args.get("id")) {
                (Some(n), _) => Pick::Index(usize::try_from(n).map_err(|_| "no such item")?),
                (None, Some(Value::String(id))) => Pick::Id(id.clone()),
                _ => return Err("give `index` or `id`".to_owned()),
            };
            Control::Item(
                pick,
                args.get("play").and_then(Value::as_bool).unwrap_or(false),
            )
        }
        other => return Err(format!("{other} is not one of the playhead's controls")),
    };
    Ok((c, source_arg(args)))
}

/// Runs `action` with `args` for `caller` on the app's deck.
///
/// What every runtime's call comes to. A control answers the state of the source it
/// moved; `PLAYHEAD_STATE` a source's state (the caller's own, or for the
/// app the active one; `null` when there is none); `PLAYHEAD_SOURCES`
/// every source.
///
/// # Errors
///
/// The contract's refusals ([`Deck::apply`]) and bad arguments.
pub fn ask(caller: &Caller, action: &str, args: &Value) -> Result<Value, String> {
    let now = now_ms();
    match action {
        "PLAYHEAD_STATE" => {
            return with(|d| {
                let id = match d.target(caller, source_arg(args).as_deref()) {
                    Ok(id) => id,
                    Err(e) if e == "no source" => return Ok(Value::Null),
                    Err(e) => return Err(e),
                };
                Ok(d.state(&id, now).unwrap_or(Value::Null))
            });
        }
        "PLAYHEAD_SOURCES" => return Ok(with(|d| d.listing())),
        _ => {}
    }
    let (c, source) = if action == "PLAYHEAD_TOGGLE" {
        // Play, or pause what plays: one press for a primary button.
        let source = source_arg(args);
        let on = with(|d| {
            d.target(caller, source.as_deref())
                .map(|id| d.source(&id).is_some_and(|s| s.transport.playing))
        })?;
        (if on { Control::Pause } else { Control::Play }, source)
    } else {
        control(action, args)?
    };
    let (state, also) = with(|d| {
        let before = d.active().map(str::to_owned);
        let id = d.apply(caller, c, source.as_deref(), now)?;
        let state = d
            .state(&id, now)
            .unwrap_or_else(|| json!({ "source": id, "removed": true }));
        // A source that lost or took the bar by this hears of it too.
        let after = d.active().map(str::to_owned);
        let also = if before == after {
            None
        } else {
            [before, after]
                .into_iter()
                .flatten()
                .find(|other| *other != id)
                .and_then(|other| d.state(&other, now))
        };
        Ok::<_, String>((state, also))
    })?;
    changed(&state);
    if let Some(a) = also {
        changed(&a);
    }
    Ok(state)
}

/// With nothing heard: past its item's end, the active source goes on
/// (the app calls it a few times a second).
pub fn tick() {
    let now = now_ms();
    let state = with(|d| {
        d.tick(now)
            .then(|| d.active().and_then(|a| d.state(a, now)))
            .flatten()
    });
    if let Some(s) = state {
        changed(&s);
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn every_runtimes_call_comes_to_one_door() {
        // The one test that owns the global deck.
        let heard = Arc::new(Mutex::new(Vec::<String>::new()));
        let h = heard.clone();
        on_change(move |s| {
            h.lock().unwrap().push(format!(
                "{} {}",
                s["source"].as_str().unwrap_or("?"),
                s["playing"]
            ));
        });
        let studio = Caller::Extension("studio".into());
        let set = json!({"name": "Studio", "queue": [{"id": "a", "title": "A", "duration": 10}]});
        let s = ask(&studio, "PLAYHEAD_SET", &set).unwrap();
        assert_eq!(s["name"], "Studio");
        assert_eq!(
            ask(&studio, "PLAYHEAD_PLAY", &json!({})).unwrap()["playing"],
            true
        );
        assert_eq!(
            ask(&Caller::App, "PLAYHEAD_STATE", &json!({})).unwrap()["source"],
            "studio"
        );
        let x = Caller::Extension("x".into());
        assert_eq!(ask(&x, "PLAYHEAD_STATE", &json!({})).unwrap(), Value::Null);
        assert_eq!(
            ask(&x, "PLAYHEAD_PAUSE", &json!({"source": "studio"})).unwrap_err(),
            "not your source"
        );
        assert!(ask(&studio, "VAULT_READ", &json!({})).is_err());
        // One press plays, the next pauses.
        assert!(playing("studio"));
        assert_eq!(
            ask(&studio, "PLAYHEAD_TOGGLE", &json!({})).unwrap()["playing"],
            false
        );
        assert_eq!(
            ask(&Caller::App, "PLAYHEAD_TOGGLE", &json!({})).unwrap()["playing"],
            true
        );
        assert!(ask(&studio, "PLAYHEAD_SEEK", &json!({})).is_err());
        // A second source playing takes the bar; the first hears it paused.
        let memo = Caller::Extension("memo".into());
        ask(
            &memo,
            "PLAYHEAD_SET",
            &json!({"queue": [{"id": "m", "title": "M", "duration": 5}], "play": true}),
        )
        .unwrap();
        assert_eq!(
            ask(&Caller::App, "PLAYHEAD_SOURCES", &json!({})).unwrap()["active"],
            "memo"
        );
        let log = heard.lock().unwrap().clone();
        assert_eq!(
            log,
            [
                "studio false",
                "studio true",
                "studio false",
                "studio true",
                "memo true",
                "studio false"
            ]
        );
        ask(&memo, "PLAYHEAD_SET", &json!({"queue": []})).unwrap();
        ask(&studio, "PLAYHEAD_SET", &json!({"queue": []})).unwrap();
        assert_eq!(
            ask(&Caller::App, "PLAYHEAD_STATE", &json!({})).unwrap(),
            Value::Null
        );
    }
}
