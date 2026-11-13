//! The Now Playing bar (#589): the shell's own surface over the app's
//! deck, the same on a desk, a phone and in the web app.
//!
//! It shows the active source: its item's art, title and subtitle,
//! previous, play/pause and next, a position line to seek on, and the
//! source's name, which lists every source when there is more than one
//! (picking one is `PLAYHEAD_ACTIVATE`). It draws nothing while no source
//! has a queue. Every press is the app's own call ([`crate::ask`] as
//! [`Caller::App`]), the same an extension's view makes for its own.
//!
//! A phone gets one compact row above its tab bar (a thin position line
//! on its top edge, the art, the title, play/pause and next); a press on
//! the title opens the rest under it. A desk gets one row with all of it.

use architect_shell::{FormFactor, use_shell};
use dioxus::prelude::*;
use lucide_dioxus::{AudioLines, ChevronsUpDown, Pause, Play, SkipBack, SkipForward};
use serde_json::{Value, json};

use crate::{Caller, Item, now_ms};

/// One source as the bar shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Showing {
    /// Its extension's id.
    pub source: String,
    /// Its name.
    pub name: String,
    /// The current item.
    pub item: Option<Item>,
    /// Its place, and how many there are.
    pub index: usize,
    /// How many items.
    pub tracks: usize,
    /// Whether it plays.
    pub playing: bool,
    /// Seconds in, when read.
    pub time: f64,
    /// The item's length, seconds.
    pub duration: f64,
}

/// The deck as the bar shows it.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Now {
    /// The deck's revision when read.
    pub revision: u64,
    /// The active source.
    pub active: Option<Showing>,
    /// Every source: its id and name.
    pub sources: Vec<(String, String)>,
}

/// The deck now.
#[must_use]
pub fn read() -> Now {
    let now = now_ms();
    crate::with(|d| Now {
        revision: d.revision(),
        active: d.current().map(|s| Showing {
            source: s.id.clone(),
            name: s.name.clone(),
            item: s.item().cloned(),
            index: s.index,
            tracks: s.queue.len(),
            playing: s.transport.playing,
            time: s.time(now),
            duration: s.duration(),
        }),
        sources: d
            .sources()
            .iter()
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect(),
    })
}

/// The deck, read again on each change and four times a second while it
/// plays. It also moves a source on at its item's end when nothing heard
/// does ([`crate::tick`]), so the app mounts it once (the bar does).
pub fn use_playhead() -> Signal<Now> {
    let mut now = use_signal(read);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(250)).await;
            crate::tick();
            let was = now.peek().clone();
            let playing = was.active.as_ref().is_some_and(|a| a.playing);
            if playing || crate::revision() != was.revision {
                let fresh = read();
                if fresh != was {
                    now.set(fresh);
                }
            }
        }
    });
    now
}

/// The app's own press: `action` with `args` on the active source (or the
/// one `args` names).
pub fn press(action: &str, args: &Value) {
    let _ = crate::ask(&Caller::App, action, args);
}

/// `m:ss`.
#[must_use]
pub fn clock_text(t: f64) -> String {
    let t = t.max(0.0).floor();
    let m = (t / 60.0).floor();
    format!("{m}:{:02}", m.mul_add(-60.0, t))
}

/// The Now Playing bar. `art` turns an item's art (a vault path) into an
/// image's URL, where the app can; without it, or without art, a mark.
#[component]
pub fn NowPlayingBar(#[props(default)] art: Option<Callback<String, Option<String>>>) -> Element {
    let shell = use_shell();
    let t = shell.theme.clone();
    let phone = shell.screen.form() == FormFactor::Phone;
    let now = use_playhead();
    let mut open = use_signal(|| false);
    let mut menu = use_signal(|| false);
    let mut line_w = use_signal(|| 0.0_f64);
    let Some(s) = now().active else {
        return rsx! {};
    };
    let Some(item) = s.item.clone() else {
        return rsx! {};
    };
    let fraction = if s.duration > 0.0 {
        (s.time / s.duration).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pct = fraction * 100.0;
    let picture = item.art.clone().and_then(|a| art.and_then(|f| f.call(a)));
    let many = now().sources.len() > 1;
    let sources = now().sources;
    let playing = s.playing;
    let (prev_ok, next_ok) = (
        s.index > 0 || s.time > 0.0,
        s.index.saturating_add(1) < s.tracks,
    );
    let size = if phone { 40 } else { 44 };
    let ink = t.ink.to_string();
    let ink3 = t.ink_3.to_string();
    let duration = s.duration;
    let seek = move |e: MouseEvent| {
        let w = *line_w.peek();
        if w > 0.0 && duration > 0.0 {
            let at = (e.element_coordinates().x / w).clamp(0.0, 1.0) * duration;
            press("PLAYHEAD_SEEK", &json!({ "t": at }));
        }
    };
    let measure = move |e: MountedEvent| async move {
        if let Ok(r) = e.data().get_client_rect().await {
            line_w.set(r.size.width);
        }
    };
    let art_box = rsx! {
        div {
            "data-testid": "now-playing-art",
            style: "flex-shrink: 0; width: {size}px; height: {size}px; border-radius: 6px; overflow: hidden; \
                    display: flex; align-items: center; justify-content: center; \
                    background: {t.active}; color: {t.ink_3};",
            if let Some(src) = picture {
                img { src: "{src}", alt: "", style: "width: 100%; height: 100%; object-fit: cover;" }
            } else {
                AudioLines { size: 18, color: ink3.clone() }
            }
        }
    };
    let words = rsx! {
        div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px;",
            span {
                "data-testid": "now-playing-title",
                style: "font-size: 14px; font-weight: 600; color: {t.ink}; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                "{item.title}"
            }
            span {
                "data-testid": "now-playing-subtitle",
                style: "font-size: 12px; color: {t.ink_3}; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                if let Some(sub) = item.subtitle.clone() { "{sub}" }
                if item.subtitle.is_some() && phone { " · " }
                if phone { "{s.name}" }
            }
        }
    };
    let (prev_fade, next_fade) = (
        if prev_ok { 1.0 } else { 0.35 },
        if next_ok { 1.0 } else { 0.35 },
    );
    let pointer = if many { "pointer" } else { "default" };
    let round = "border: 0; background: transparent; padding: 0; width: 36px; height: 36px; border-radius: 999px; \
                 display: flex; align-items: center; justify-content: center; cursor: pointer; flex-shrink: 0;";
    let previous = rsx! {
        button {
            "data-testid": "now-playing-previous", "aria-label": "Previous",
            style: "{round} opacity: {prev_fade};",
            onclick: move |e: MouseEvent| { e.stop_propagation(); press("PLAYHEAD_PREVIOUS", &json!({})); },
            SkipBack { size: 18, color: ink.clone() }
        }
    };
    let toggle = rsx! {
        button {
            "data-testid": "now-playing-toggle",
            "aria-label": if playing { "Pause" } else { "Play" },
            "data-playing": "{playing}",
            style: "{round} background: {t.accent};",
            onclick: move |e: MouseEvent| {
                e.stop_propagation();
                press(if playing { "PLAYHEAD_PAUSE" } else { "PLAYHEAD_PLAY" }, &json!({}));
            },
            if playing {
                Pause { size: 18, color: t.accent_ink.to_string() }
            } else {
                Play { size: 18, color: t.accent_ink.to_string() }
            }
        }
    };
    let next = rsx! {
        button {
            "data-testid": "now-playing-next", "aria-label": "Next",
            style: "{round} opacity: {next_fade};",
            onclick: move |e: MouseEvent| { e.stop_propagation(); press("PLAYHEAD_NEXT", &json!({})); },
            SkipForward { size: 18, color: ink }
        }
    };
    let line = rsx! {
        div { style: "flex: 1; min-width: 80px; display: flex; align-items: center; gap: 8px; \
                      font-size: 11px; color: {t.ink_3}; font-variant-numeric: tabular-nums;",
            span { "data-testid": "now-playing-time", "{clock_text(s.time)}" }
            div {
                "data-testid": "now-playing-line",
                role: "slider", "aria-label": "Position",
                "aria-valuemin": "0", "aria-valuemax": "{s.duration:.0}", "aria-valuenow": "{s.time:.1}",
                style: "position: relative; flex: 1; height: 16px; cursor: pointer;",
                onmounted: measure,
                onclick: seek,
                div { style: "position: absolute; left: 0; right: 0; top: 7px; height: 3px; border-radius: 2px; background: {t.rule};" }
                div { style: "position: absolute; left: 0; width: {pct}%; top: 7px; height: 3px; border-radius: 2px; background: {t.accent};" }
            }
            span { "{clock_text(s.duration)}" }
        }
    };
    let switcher = rsx! {
        div { style: "position: relative; flex-shrink: 0;",
            button {
                "data-testid": "now-playing-source",
                "aria-label": "Source: {s.name}",
                style: "border: 1px solid {t.rule}; background: transparent; color: {t.ink_3}; border-radius: 999px; \
                        padding: 4px 10px; font: inherit; font-size: 12px; display: flex; align-items: center; gap: 4px; \
                        cursor: {pointer}; max-width: 160px;",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    if many {
                        let o = *menu.peek();
                        menu.set(!o);
                    }
                },
                span { style: "white-space: nowrap; overflow: hidden; text-overflow: ellipsis;", "{s.name}" }
                if many {
                    ChevronsUpDown { size: 12, color: ink3 }
                }
            }
            if menu() && many {
                div {
                    "data-testid": "now-playing-sources",
                    role: "menu",
                    style: "position: absolute; right: 0; bottom: calc(100% + 6px); min-width: 180px; z-index: 30; \
                            background: {t.sheet}; border: 1px solid {t.rule}; border-radius: 10px; padding: 4px; \
                            box-shadow: {t.shadow_float}; display: flex; flex-direction: column;",
                    for (id, name) in sources {
                        {
                        let bg = if id == s.source { t.active.to_string() } else { "transparent".to_owned() };
                        rsx! {
                        button {
                            key: "{id}",
                            role: "menuitem",
                            "data-source": "{id}",
                            style: "border: 0; text-align: left; padding: 8px 10px; border-radius: 6px; font: inherit; font-size: 13px; \
                                    cursor: pointer; color: {t.ink}; background: {bg};",
                            onclick: {
                                move |e: MouseEvent| {
                                e.stop_propagation();
                                menu.set(false);
                                press("PLAYHEAD_ACTIVATE", &json!({ "source": id }));
                            }},
                            "{name}"
                        }
                        }
                        }
                    }
                }
            }
        }
    };
    if phone {
        rsx! {
            div {
                "data-testid": "now-playing",
                "data-source": "{s.source}",
                "data-playing": "{playing}",
                "data-time": "{s.time:.1}",
                "data-index": "{s.index}",
                "aria-label": "Now playing",
                style: "position: relative; display: flex; flex-direction: column; background: {t.sheet}; \
                        border-top: 1px solid {t.rule}; color: {t.ink};",
                // The position, on the row's top edge.
                div { style: "position: absolute; left: 0; top: -1px; height: 2px; width: {pct}%; background: {t.accent};" }
                div {
                    style: "display: flex; align-items: center; gap: 10px; padding: 8px 12px; cursor: pointer;",
                    onclick: move |_| { let o = *open.peek(); open.set(!o); },
                    {art_box}
                    {words}
                    {toggle}
                    {next}
                }
                if open() {
                    div { style: "display: flex; align-items: center; gap: 8px; padding: 0 12px 10px;",
                        {previous}
                        {line}
                        {switcher}
                    }
                }
            }
        }
    } else {
        rsx! {
            div {
                "data-testid": "now-playing",
                "data-source": "{s.source}",
                "data-playing": "{playing}",
                "data-time": "{s.time:.1}",
                "data-index": "{s.index}",
                "aria-label": "Now playing",
                style: "flex-shrink: 0; display: flex; align-items: center; gap: 12px; padding: 8px 14px; \
                        background: {t.sheet}; border-top: 1px solid {t.rule}; color: {t.ink};",
                div { style: "display: flex; align-items: center; gap: 10px; flex: 0 1 280px; min-width: 0;",
                    {art_box}
                    {words}
                }
                div { style: "display: flex; align-items: center; gap: 4px;",
                    {previous}
                    {toggle}
                    {next}
                }
                {line}
                {switcher}
            }
        }
    }
}
