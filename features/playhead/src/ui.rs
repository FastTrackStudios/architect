//! The Now Playing bar (#589): the shell's own surface over the app's
//! deck, the same on a desk, a phone and in the web app.
//!
//! It is a share page's player card (`architect-sites`' `site-player-card`):
//! the active source's art, its item's title over its subtitle, the
//! source's two slots ([`Slot`]: Click, and `^` for the detail), then
//! play, with its progress a thin line along the card's bottom edge. The
//! detail, opened by `^` or a press on the song, is previous and next,
//! the position line to seek on, and the source's name, which lists every
//! source when there is more than one (picking one is `PLAYHEAD_ACTIVATE`).
//! It draws nothing while no source has a queue. Every press is the app's
//! own call ([`crate::ask`] as [`Caller::App`]), the same an extension's
//! view makes for its own; Click is the app's ([`crate::set_click`]),
//! offered where the app sounds one ([`crate::clicks`]).
//!
//! The same card on a phone (above its tab bar) and a desk (at the page's
//! foot).
//!
//! Every button is a 44 px target; the position line is a slider a
//! keyboard reaches (arrows: 5 s, Home/End). When the item has media
//! nobody hears here, the bar says so ("Not heard here") rather than
//! looking like it plays, and an item it had to pass over or stop at
//! ("Can't be played": media with no known length) is named.

use architect_shell::{FormFactor, use_shell};
use dioxus::prelude::*;
use lucide_dioxus::{
    AudioLines, ChevronDown, ChevronUp, ChevronsUpDown, Pause, Play, SkipBack, SkipForward,
};
use serde_json::{Value, json};

use crate::{Caller, Item, Slot, now_ms};

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
    /// The item has media nobody hears here (the deck's silent clock).
    pub unheard: bool,
    /// The item cannot be played: media nobody hears and no known length.
    pub unplayable: bool,
    /// What it asks the bar to draw beside play.
    pub slots: Vec<Slot>,
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
            unheard: d
                .state(&s.id, now)
                .is_some_and(|v| v.get("unheard") == Some(&Value::Bool(true))),
            unplayable: s.item().is_some_and(|i| s.unplayable.contains(&i.id)),
            slots: s.slots.clone(),
        }),
        sources: d
            .sources()
            .iter()
            .map(|s| (s.id.clone(), s.name.clone()))
            .collect(),
    })
}

/// The deck, read again on each change and while it plays.
///
/// It looks ten times a second. It also moves a source on at its item's
/// end when nothing heard does ([`crate::tick`]), so the app mounts it
/// once (the bar does).
pub fn use_playhead() -> Signal<Now> {
    let mut now = use_signal(read);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(100)).await;
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

/// The slots the bar draws for `asked`, and `^` always.
///
/// As asked (Click only where the app sounds one), `^` last when not
/// asked for. The detail
/// (previous, next, the position, the sources) is then always one focus
/// away, whatever a source asks (#667 M2): the song area's press is the
/// pointer's shortcut, not the only way in.
#[must_use]
pub fn slots_drawn(asked: &[Slot], clicks: bool) -> Vec<Slot> {
    let mut v: Vec<Slot> = asked
        .iter()
        .copied()
        .filter(|s| *s != Slot::Click || clicks)
        .collect();
    if !v.contains(&Slot::Open) {
        v.push(Slot::Open);
    }
    v
}

/// A metronome: its body, its pendulum (the share page's Click).
#[component]
fn Metronome(color: String) -> Element {
    rsx! {
        svg { view_box: "0 0 24 24", width: "20", height: "20", fill: "{color}", "aria-hidden": "true",
            path { fill_rule: "evenodd", d: "M9 2.5h6l4.6 19H4.4zm1.3 1.8-3.5 15.4h10.4L13.7 4.3z" }
            path { d: "m11.2 17.6 6.3-11 1.5.9-6.3 11z" }
            path { d: "M6.3 17.2h11.4v2.5H6.3z" }
        }
    }
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
    let mut click = use_signal(crate::click);
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
    let slots = slots_drawn(&s.slots, crate::clicks());
    let has_tempo = item.tempo.is_some();
    let ink = t.ink.to_string();
    let ink3 = t.ink_3.to_string();
    let duration = s.duration;
    let time_now = s.time;
    // The keyboard's way along the line: 5 s a press, Home and End.
    let keys = move |e: KeyboardEvent| {
        if duration <= 0.0 {
            return;
        }
        let to = match e.key() {
            Key::Named(NamedKey::ArrowRight | NamedKey::ArrowUp) => (time_now + 5.0).min(duration),
            Key::Named(NamedKey::ArrowLeft | NamedKey::ArrowDown) => (time_now - 5.0).max(0.0),
            Key::Named(NamedKey::Home) => 0.0,
            Key::Named(NamedKey::End) => duration,
            _ => return,
        };
        e.prevent_default();
        press("PLAYHEAD_SEEK", &json!({ "t": to }));
    };
    let seek = move |e: MouseEvent| {
        e.stop_propagation();
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
    let round = "border: 0; background: transparent; color: inherit; padding: 0; width: 44px; height: 44px; border-radius: 999px; \
                 display: flex; align-items: center; justify-content: center; cursor: pointer; flex-shrink: 0;";
    let art_box = rsx! {
        div {
            "data-testid": "now-playing-art",
            style: "flex-shrink: 0; width: 40px; height: 40px; border-radius: 4px; overflow: hidden; \
                    display: flex; align-items: center; justify-content: center; \
                    background: {t.active}; color: {t.ink_3};",
            if let Some(src) = picture {
                img { src: "{src}", alt: "", style: "width: 100%; height: 100%; object-fit: cover;" }
            } else {
                AudioLines { size: 20, color: ink3.clone() }
            }
        }
    };
    let words = rsx! {
        div { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; padding: 0 10px;",
            span {
                "data-testid": "now-playing-title",
                style: "font-size: 14px; font-weight: 600; color: {t.ink}; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                "{item.title}"
            }
            span {
                "data-testid": "now-playing-subtitle",
                style: "font-size: 12.5px; color: {t.ink_3}; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;",
                if let Some(sub) = item.subtitle.clone() { "{sub}" }
                if item.subtitle.is_some() { " · " }
                "{s.name}"
            }
            if s.unplayable || s.unheard {
                span {
                    "data-testid": "now-playing-unheard",
                    role: "status",
                    style: "font-size: 11px; font-weight: 600; color: {t.ink_3};",
                    if s.unplayable { "Can't be played" } else { "Not heard here" }
                }
            }
        }
    };
    // The slots: off, an icon; on, the icon on a disc of the accent.
    let accent = t.accent.to_string();
    let accent_ink = t.accent_ink.to_string();
    let slot_buttons = slots.iter().map(|slot| match slot {
        Slot::Click => {
            let on = click();
            let (fg, bg, fade) = if on {
                (accent_ink.clone(), format!("radial-gradient(circle, {accent} 0 15px, transparent 15.5px)"), 1.0)
            } else {
                (ink.clone(), "transparent".to_owned(), if has_tempo { 0.8 } else { 0.32 })
            };
            rsx! {
                button {
                    key: "{slot:?}",
                    "data-testid": "now-playing-click",
                    "aria-label": "Click",
                    "aria-pressed": "{on}",
                    title: if has_tempo { "Click: a beat with the song" } else { "Click: this song has no tempo" },
                    style: "{round} background: {bg}; color: {fg}; opacity: {fade};",
                    onclick: move |e: MouseEvent| {
                        e.stop_propagation();
                        let to = !*click.peek();
                        crate::set_click(to);
                        click.set(to);
                    },
                    Metronome { color: fg.clone() }
                }
            }
        }
        Slot::Open => rsx! {
            button {
                key: "{slot:?}",
                "data-testid": "now-playing-open",
                "aria-label": if open() { "Close the player" } else { "Open the player" },
                "aria-expanded": "{open()}",
                style: "{round} opacity: 0.8;",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    let o = *open.peek();
                    open.set(!o);
                },
                if open() {
                    ChevronDown { size: 22, color: ink.clone() }
                } else {
                    ChevronUp { size: 22, color: ink.clone() }
                }
            }
        },
    });
    let toggle = rsx! {
        button {
            "data-testid": "now-playing-toggle",
            "aria-label": if playing { "Pause" } else { "Play" },
            "data-playing": "{playing}",
            style: "{round}",
            onclick: move |e: MouseEvent| {
                e.stop_propagation();
                press(if playing { "PLAYHEAD_PAUSE" } else { "PLAYHEAD_PLAY" }, &json!({}));
            },
            if playing {
                Pause { size: 26, color: t.ink.to_string(), fill: t.ink.to_string() }
            } else {
                Play { size: 26, color: t.ink.to_string(), fill: t.ink.to_string() }
            }
        }
    };
    let (prev_fade, next_fade) = (
        if prev_ok { 1.0 } else { 0.35 },
        if next_ok { 1.0 } else { 0.35 },
    );
    let pointer = if many { "pointer" } else { "default" };
    let previous = rsx! {
        button {
            "data-testid": "now-playing-previous", "aria-label": "Previous",
            disabled: !prev_ok,
            style: "{round} opacity: {prev_fade};",
            onclick: move |e: MouseEvent| { e.stop_propagation(); press("PLAYHEAD_PREVIOUS", &json!({})); },
            SkipBack { size: 20, color: ink.clone() }
        }
    };
    let next = rsx! {
        button {
            "data-testid": "now-playing-next", "aria-label": "Next",
            disabled: !next_ok,
            style: "{round} opacity: {next_fade};",
            onclick: move |e: MouseEvent| { e.stop_propagation(); press("PLAYHEAD_NEXT", &json!({})); },
            SkipForward { size: 20, color: ink.clone() }
        }
    };
    let line = rsx! {
        div { style: "flex: 1; min-width: 80px; display: flex; align-items: center; gap: 8px; \
                      font-size: 11px; color: {t.ink_3}; font-variant-numeric: tabular-nums;",
            span { "data-testid": "now-playing-time", "{clock_text(s.time)}" }
            div {
                "data-testid": "now-playing-line",
                role: "slider", "aria-label": "Position", tabindex: "0",
                "aria-valuemin": "0", "aria-valuemax": "{s.duration:.0}", "aria-valuenow": "{s.time:.1}",
                "aria-valuetext": "{clock_text(s.time)} of {clock_text(s.duration)}",
                "aria-disabled": "{s.duration <= 0.0}",
                style: "position: relative; flex: 1; height: 44px; cursor: pointer;",
                onmounted: measure,
                onclick: seek,
                onkeydown: keys,
                div { style: "position: absolute; left: 0; right: 0; top: 20px; height: 4px; border-radius: 2px; background: {t.rule};" }
                div { style: "position: absolute; left: 0; width: {pct}%; top: 20px; height: 4px; border-radius: 2px; background: {t.accent};" }
            }
            span { "{clock_text(s.duration)}" }
        }
    };
    let switcher = rsx! {
        div { style: "position: relative; flex-shrink: 0;",
            button {
                "data-testid": "now-playing-source",
                "aria-label": "Source: {s.name}",
                "aria-haspopup": if many { "menu" } else { "false" },
                "aria-expanded": "{menu() && many}",
                // The press is 44 points tall; the pill drawn in it is the
                // name's own height (a 44-point outline round 12px text
                // reads as a stretched oval on a phone).
                style: "border: 0; background: transparent; color: {t.ink_3}; box-sizing: border-box; \
                        min-height: 44px; padding: 0; font: inherit; font-size: 12px; display: flex; align-items: center; \
                        cursor: {pointer}; max-width: 160px;",
                onclick: move |e: MouseEvent| {
                    e.stop_propagation();
                    if many {
                        let o = *menu.peek();
                        menu.set(!o);
                    }
                },
                span { style: "display: flex; align-items: center; gap: 4px; min-width: 0; padding: 5px 12px; \
                               border: 1px solid {t.rule}; border-radius: 999px;",
                    span { style: "white-space: nowrap; overflow: hidden; text-overflow: ellipsis;", "{s.name}" }
                    if many {
                        ChevronsUpDown { size: 12, color: ink3 }
                    }
                }
            }
            if menu() && many {
                div {
                    "data-testid": "now-playing-sources",
                    role: "menu",
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Named(NamedKey::Escape) {
                            menu.set(false);
                        }
                    },
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
                            style: "border: 0; text-align: left; min-height: 44px; padding: 8px 10px; border-radius: 6px; font: inherit; font-size: 13px; \
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
    // A card: on its gutters, its own corners and shadow, over the page.
    let (gutter, foot) = if phone { (12, 8) } else { (16, 12) };
    // Shown or not, the detail stays laid out (the line keeps its width to
    // seek by): closed, it takes no height and is out of sight and out of
    // the keyboard's way (`visibility: hidden`).
    // Each state names every property: a style's update keeps what the
    // new one leaves out.
    let detail = if open() {
        "height: auto; overflow: visible; visibility: visible; padding: 6px 8px 0;"
    } else {
        "height: 0; overflow: hidden; visibility: hidden; padding: 0 8px;"
    };
    rsx! {
        div {
            "data-testid": "now-playing",
            "data-source": "{s.source}",
            "data-playing": "{playing}",
            "data-time": "{s.time:.1}",
            "data-index": "{s.index}",
            "data-open": "{open()}",
            "aria-label": "Now playing",
            role: "region",
            style: "padding: 0 {gutter}px {foot}px; flex-shrink: 0;",
            div {
                style: "position: relative; display: flex; flex-direction: column; overflow: hidden; \
                        background: {t.sheet}; color: {t.ink}; border: 1px solid {t.rule}; border-radius: 10px; \
                        box-shadow: {t.shadow_float};",
                div {
                    "data-testid": "now-playing-detail",
                    "aria-hidden": "{!open()}",
                    style: "display: flex; flex-direction: column; {detail}",
                    // The position, the whole width; under it previous,
                    // next and the source.
                    div { style: "display: flex; align-items: center; padding: 0 4px;", {line} }
                    div { style: "display: flex; align-items: center; gap: 4px;",
                        {previous}
                        {next}
                        span { style: "flex: 1;" }
                        {switcher}
                    }
                }
                div {
                    style: "display: flex; align-items: center; min-height: 60px; padding: 8px 4px 8px 8px; cursor: pointer;",
                    // A press on the song (anywhere but its buttons) opens
                    // the detail, as a share card opens its view.
                    onclick: move |_| { let o = *open.peek(); open.set(!o); },
                    {art_box}
                    {words}
                    for b in slot_buttons { {b} }
                    {toggle}
                }
                // Its progress: a thin line along its bottom edge, inside its corners.
                div { "aria-hidden": "true",
                    style: "position: absolute; left: 8px; right: 8px; bottom: 0; height: 2px; border-radius: 1px; background: {t.rule};",
                    div { style: "height: 100%; width: {pct}%; border-radius: 1px; background: {t.ink};" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_draws_the_slots_asked_click_only_where_heard_else_open() {
        assert_eq!(slots_drawn(&[], true), [Slot::Open]);
        assert_eq!(
            slots_drawn(&[Slot::Click, Slot::Open], true),
            [Slot::Click, Slot::Open]
        );
        assert_eq!(slots_drawn(&[Slot::Click, Slot::Open], false), [Slot::Open]);
        assert_eq!(
            slots_drawn(&[Slot::Open, Slot::Click], true),
            [Slot::Open, Slot::Click]
        );
        // #667 M2: `^` is always there, so the detail is reachable from
        // the keyboard whatever a source asks.
        assert_eq!(slots_drawn(&[Slot::Click], false), [Slot::Open]);
        assert_eq!(slots_drawn(&[Slot::Click], true), [Slot::Click, Slot::Open]);
    }
}
