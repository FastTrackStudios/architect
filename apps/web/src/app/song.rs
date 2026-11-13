//! A song note's extension views (#495): the node's island (Studio's
//! progress bar, transport and lyrics) in frames of their own beside the
//! note (`frames`), driven by the app's playhead.
//!
//! The playhead is the app's: a clock over the song's length (its
//! `duration` property, else past its last timed line), played, paused
//! and sought by the app's own controls and by the views' asks (checked
//! in `frames`, answered here). The note's timed lyrics are its ` ```lrc `
//! fence (`crate::ext_host::lyrics`). The app's own controls and lyrics
//! show until a view is ready and for good if it dies; a node without an
//! island leaves them in its place.

use std::rc::Rc;

use architect_shell::use_shell;
use dioxus::prelude::*;
use serde_json::{Value, json};

use super::frames::{Bundle, ExtensionFrame, FrameTheme, use_island_of};
use crate::ext_host::{self, Intent, Line, Section};

pub(super) fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map_or(0.0, |p| p.now())
}

/// Where the song is: `time` seconds at `at` (ms), playing or not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Clock {
    pub(super) time: f64,
    pub(super) at: f64,
    pub(super) playing: bool,
}

impl Clock {
    pub(super) fn time(self, duration: f64) -> f64 {
        if self.playing {
            (self.time + (now_ms() - self.at) / 1000.0).min(duration)
        } else {
            self.time
        }
    }
}

const COLORS: &[&str] = &[
    "#2563eb", "#16a34a", "#d97706", "#9333ea", "#db2777", "#0891b2",
];

fn clock_text(t: f64) -> String {
    let t = t.max(0.0).floor();
    let m = (t / 60.0).floor();
    format!("{m}:{:02}", m.mul_add(-60.0, t))
}

/// The views' box: two columns on a wide pane (the note, then the views
/// beside it), one on a narrow one (the views above the note).
pub const LAYOUT: &str = "
.web-song { display: flex; flex-direction: column; gap: 10px; margin: 0 0 20px; }
.web-song-lyrics { height: 260px; border-radius: 10px; overflow: hidden; }
.web-song-own { display: flex; align-items: center; gap: 10px; font-size: 13px; }
.web-song-own button { border: 0; border-radius: 999px; padding: 6px 14px; font: inherit; font-weight: 600; }
.web-song-own input { flex: 1; }
.web-song-words { list-style: none; margin: 0; padding: 12px; height: 100%; box-sizing: border-box; overflow: auto; font-size: 15px; line-height: 1.8; }
.web-note-row { display: flex; flex-direction: column; }
@container note (min-width: 1000px) {
  .web-note-row { flex-direction: row-reverse; align-items: flex-start; gap: 28px; }
  .web-note-row > .web-song { position: sticky; top: 0; flex: 0 0 380px; }
  .web-note-row > .web-note { flex: 1 1 auto; }
}
";

/// The theme a frame is handed: the shell's colours and its font.
pub(super) fn frame_theme() -> FrameTheme {
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

/// A song note's views: `title`, by `artist`, its `body`'s timed lyrics,
/// `duration` its `duration` property. With `only`, that one view of
/// island `island` (a favourite's `{view}`, #607) filling its pane, the
/// app's own control or lyrics in its place until it is ready.
#[component]
pub fn SongViews(
    title: String,
    artist: String,
    body: String,
    duration: Option<String>,
    #[props(default)] island: Option<String>,
    #[props(default)] only: Option<String>,
) -> Element {
    let t = use_shell().theme;
    let (lines, sections) = use_hook(|| {
        let (lines, _) = ext_host::lyrics(&body, 0.0);
        let d = ext_host::duration(duration.as_deref(), &lines);
        ext_host::lyrics(&body, d)
    });
    let length = ext_host::duration(duration.as_deref(), &lines);
    let mut clock = use_signal(|| Clock {
        time: 0.0,
        at: now_ms(),
        playing: false,
    });
    let mut time = use_signal(|| 0.0_f64);
    // The playhead: a tick while it plays, stopping at the end.
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(100)).await;
            let c = *clock.peek();
            if !c.playing {
                continue;
            }
            let now = c.time(length);
            time.set(now);
            if now >= length {
                clock.set(Clock {
                    time: length,
                    at: now_ms(),
                    playing: false,
                });
            }
        }
    });
    let mut act = move |i: Intent| {
        let c = *clock.peek();
        let at = c.time(length);
        let next = match i {
            Intent::Play => Clock {
                time: if at >= length { 0.0 } else { at },
                at: now_ms(),
                playing: true,
            },
            Intent::Pause => Clock {
                time: at,
                at: now_ms(),
                playing: false,
            },
            Intent::Seek(s) => Clock {
                time: s.min(length),
                at: now_ms(),
                playing: c.playing,
            },
            // One song: the previous is its start, the next its end.
            Intent::Previous => Clock {
                time: 0.0,
                at: now_ms(),
                playing: c.playing,
            },
            Intent::Next => Clock {
                time: length,
                at: now_ms(),
                playing: false,
            },
            // A song note's views have no queue.
            Intent::Set(..) | Intent::Item(..) => return,
        };
        clock.set(next);
        time.set(next.time);
    };
    let sections_json: Vec<Value> = sections
        .iter()
        .enumerate()
        .map(|(n, s): (usize, &Section)| {
            let color = COLORS
                .get(n.checked_rem(COLORS.len()).unwrap_or(0))
                .copied()
                .unwrap_or("#64748b");
            json!({"name": s.name, "start": s.start, "end": s.end, "color": color})
        })
        .collect();
    let lines_json: Vec<Value> = lines
        .iter()
        .map(|l: &Line| json!({"at": l.at, "text": l.text}))
        .collect();
    let props = use_memo({
        let (title, sections_json) = (title.clone(), sections_json);
        move || {
            json!({
                "time": time(),
                "playing": clock().playing,
                "duration": length,
                "index": 0,
                "tracks": 1,
                "title": title,
                "sections": sections_json,
            })
        }
    });
    let words = use_memo({
        let (artist, lines_json) = (artist, lines_json);
        move || {
            let mut p = props();
            if let Some(o) = p.as_object_mut() {
                o.insert("lyrics".to_owned(), Value::Array(lines_json.clone()));
                o.insert("artist".to_owned(), Value::String(artist.clone()));
            }
            p
        }
    });
    let island = use_island_of(island);
    let theme = frame_theme();
    let bundle: Option<Rc<Bundle>> = island
        .read()
        .as_ref()
        .and_then(|r| r.as_ref().ok().cloned());
    let playing = clock().playing;
    let now = time();
    let own_controls = rsx! {
        div { class: "web-song-own", "data-testid": "song-own-controls", style: "color: {t.ink};",
            button {
                "data-testid": "song-own-play",
                style: "background: {t.accent}; color: {t.accent_ink};",
                onclick: move |_| act(if playing { Intent::Pause } else { Intent::Play }),
                if playing { "Pause" } else { "Play" }
            }
            input {
                r#type: "range",
                "aria-label": "Position",
                min: "0",
                max: "{length}",
                step: "0.1",
                value: "{now}",
                oninput: move |e: FormEvent| {
                    if let Ok(s) = e.value().parse::<f64>() {
                        act(Intent::Seek(s));
                    }
                },
            }
            span { style: "font-variant-numeric: tabular-nums; color: {t.ink_3};",
                "{clock_text(now)} / {clock_text(length)}"
            }
        }
    };
    let current = lines.iter().rposition(|l| l.at <= now);
    let own_words = rsx! {
        ul { class: "web-song-words", "data-testid": "song-own-lyrics", style: "color: {t.ink_3}; background: {t.sheet};",
            if lines.is_empty() {
                li { "No timed lyrics in this note." }
            }
            for (n, l) in lines.iter().enumerate() {
                li {
                    key: "{n}",
                    style: if current == Some(n) { format!("color: {}; font-weight: 600;", t.ink) } else { String::new() },
                    "{l.text}"
                }
            }
        }
    };
    let on_ask = move |i: Intent| act(i);
    if let Some(view) = only {
        let of = view.clone();
        // One view, filling the pane: the lyrics' words, any other the
        // playhead's props; the app's own in its place.
        let lyrics = bundle
            .as_ref()
            .is_some_and(|b| b.island.lyrics.as_ref() == Some(&view));
        let fallback = if lyrics { own_words } else { own_controls };
        return rsx! {
            section { class: "web-song-one", "data-testid": "ext-view-pane", "data-ext-view-of": "{of}",
                "aria-label": "{title}",
                style: "flex: 1 1 0; min-height: 0; display: flex; flex-direction: column; gap: 10px; padding: 12px 16px;",
                "data-song-time": "{now:.1}", "data-song-playing": "{playing}",
                div { style: "flex: 1 1 0; min-height: 0; border-radius: 10px; overflow: hidden;",
                    match bundle {
                        Some(b) => rsx! {
                            ExtensionFrame {
                                bundle: b, view, fill: true,
                                props: if lyrics { words } else { props },
                                duration: length, theme, on_ask, fallback,
                            }
                        },
                        None => fallback,
                    }
                }
            }
        };
    }
    rsx! {
        section { class: "web-song", "data-testid": "song-views", "aria-label": "Song",
            "data-song-time": "{now:.1}", "data-song-playing": "{playing}",
            match bundle {
                Some(b) => {
                    let progress = b.island.progress.clone();
                    let transport = b.island.transport.clone();
                    let lyrics = b.island.lyrics.clone();
                    rsx! {
                        if let Some(v) = progress {
                            ExtensionFrame {
                                bundle: b.clone(), view: v, props, duration: length,
                                theme: theme.clone(), on_ask, fallback: rsx! {},
                            }
                        }
                        if let Some(v) = transport {
                            ExtensionFrame {
                                bundle: b.clone(), view: v, props, duration: length,
                                theme: theme.clone(), on_ask, fallback: own_controls,
                            }
                        } else {
                            {own_controls}
                        }
                        div { class: "web-song-lyrics",
                            if let Some(v) = lyrics {
                                ExtensionFrame {
                                    bundle: b, view: v, fill: true, props: words, duration: length,
                                    theme, on_ask, fallback: own_words,
                                }
                            } else {
                                {own_words}
                            }
                        }
                    }
                }
                None => rsx! {
                    {own_controls}
                    div { class: "web-song-lyrics", {own_words} }
                },
            }
        }
    }
}
