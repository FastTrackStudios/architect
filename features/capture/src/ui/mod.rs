//! The window side (feature `ui`): the composer, the inbox deck, and the
//! capture window.
//!
//! [`use_capture`] puts `CAPTURE_*` and `INBOX_*` on the app's list and
//! gives back a [`CaptureUi`] (in context too): open the composer, the
//! deck or the floating window from a key, a button or an action.
//! [`CaptureLayer`], mounted once over the window, draws what is open:
//!
//! - the [`Composer`], ported from Task (#445): a full-screen sheet on a
//!   phone whose properties ride the keyboard as one scrolling bar, a
//!   card with its chips in a row on a desk; a title read as it is typed
//!   (`@` people and contexts, `#` labels, `[[Project]]`, dates, repeats,
//!   `!priority` as live chips; a chip set by hand wins), a More menu, the
//!   kind at the top a menu of `[[capture_targets]]`, "Create more", ⌘↩;
//! - [`QuickAdd`], the inline bar with a live preview of what it parsed;
//! - the [`InboxDeck`]: each capture once, its targets, Later, Discard;
//! - the capture window: the composer alone in a small OS window of its
//!   own, where the platform gives one (`architect_workspace::Popout::page`;
//!   feature `native` raises it over every app and puts it where the
//!   person is looking), else a card.

mod chips;
mod composer;
mod picker;
mod quick_add;
mod style;

pub use chips::{
    AssigneeChip, Chip, Hotkey, LabelsChip, PRIORITIES, PersonDot, PriorityChip, PriorityIcon,
    ProjectChip, STATUSES, StatusChip, StatusIcon, canonical_priority, canonical_status,
    label_color,
};
pub use composer::{
    CaptureKind, Composer, Hand, Trigger, drop_last_word, kind_from_text, offers_process_now,
    project_title, resolved_kind, trigger_at_end,
};
pub use picker::{PickOption, PropertyPicker, filtered, match_rank};
pub use quick_add::{QuickAdd, parsed_bits};
pub use style::{CSS, CaptureCss, Look, use_look, wash};

use std::rc::Rc;
use std::sync::Arc;

use architect::action::{ActionCaller, ActionList};
use architect_shell::{ShellTheme, try_use_shell};
use dioxus::prelude::*;
use futures_util::StreamExt as _;
use serde_json::{Map, Value};

use crate::actions::{self, Capturer, Config, Open, Opener};
use crate::inbox::Item;
use crate::parse::keys as k;
use crate::target::{self, Target};

/// The capture window's id among the desk's windows.
pub const WINDOW: &str = "capture";

/// The capture window's size, logical points.
pub const WINDOW_SIZE: (f64, f64) = (560.0, 300.0);

/// The composer, the deck and the capture window: what is open, and the
/// [`Capturer`] they write through. Copy; in context below
/// [`use_capture`].
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct CaptureUi {
    /// The composer over the window: its text and target.
    sheet: Signal<Option<(String, Option<String>)>>,
    deck: Signal<bool>,
    /// The capture window, its text.
    window: Signal<Option<String>>,
    /// Bumped after every change made here, for lists to read again.
    revision: Signal<u64>,
    capturer: CopyValue<Capturer>,
}

impl CaptureUi {
    /// Opens the composer over the window, filled with `text`.
    pub fn compose(mut self, text: &str) {
        self.sheet.set(Some((text.to_owned(), None)));
    }

    /// Opens the floating capture window (the global shortcut's).
    pub fn window(mut self, text: &str) {
        self.window.set(Some(text.to_owned()));
    }

    /// Opens the inbox deck.
    pub fn inbox(mut self) {
        self.deck.set(true);
    }

    /// Closes the composer over the window.
    pub fn close(mut self) {
        self.sheet.set(None);
    }

    /// Whether the composer, the deck or the capture window is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.sheet.read().is_some() || *self.deck.read() || self.window.read().is_some()
    }

    #[must_use]
    pub fn capturer(&self) -> Capturer {
        self.capturer.read().clone()
    }

    /// Bumped after every change made from here.
    #[must_use]
    pub fn revision(&self) -> u64 {
        *self.revision.read()
    }

    fn changed(mut self) {
        let n = *self.revision.peek();
        self.revision.set(n.wrapping_add(1));
    }
}

/// Puts `CAPTURE_*` and `INBOX_*` on `list` (once) and gives back the
/// window's handle on them, in context for everything below.
#[must_use]
pub fn use_capture(list: &Arc<ActionList>, config: Config) -> CaptureUi {
    let mut sheet = use_signal(|| None);
    let mut deck = use_signal(|| false);
    let mut window = use_signal(|| None);
    let revision = use_signal(|| 0_u64);
    // An action may run on any thread: what it asks comes here.
    let asks = use_coroutine(move |mut rx: UnboundedReceiver<Open>| async move {
        while let Some(open) = rx.next().await {
            match open {
                Open::Composer { text, target } => sheet.set(Some((text, target))),
                Open::Window { text } => window.set(Some(text)),
                Open::Inbox => deck.set(true),
                Open::Close => {
                    sheet.set(None);
                    deck.set(false);
                }
            }
        }
    });
    let capturer = use_hook(|| {
        let tx = asks.tx();
        let opener: Opener = Arc::new(move |open| {
            let _ = tx.unbounded_send(open);
        });
        // Taken already (a second window on the same list): the first's
        // actions answer, this window still captures through its own.
        let cap = actions::register(list, config.clone(), Some(opener))
            .unwrap_or_else(|_| Capturer::new(list, config));
        CopyValue::new(cap)
    });
    let ui = CaptureUi {
        sheet,
        deck,
        window,
        revision,
        capturer,
    };
    use_context_provider(|| ui)
}

/// The [`CaptureUi`] above, if any.
#[must_use]
pub fn try_use_capture() -> Option<CaptureUi> {
    try_consume_context::<CaptureUi>()
}

fn theme() -> ShellTheme {
    try_use_shell().map_or_else(
        || ShellTheme::from_resolved(&architect_theme::use_resolved()),
        |s| s.theme,
    )
}

fn phone() -> bool {
    style::phone()
}

/// What is open, over the window: mount it once at the root.
#[component]
pub fn CaptureLayer(capture: CaptureUi) -> Element {
    let t = theme();
    let os = try_consume_context::<architect_workspace::Windows>();
    let sheet = capture.sheet.read().clone();
    let window = capture.window.read().clone();
    let deck = *capture.deck.read();
    // Task's composer frame: on a phone the sheet is the screen; on a desk
    // a scrim, and a card 10vh down (`md:max-w-2xl md:max-h-[80vh]
    // md:rounded-xl md:border md:shadow-2xl`). The card does not clip, so
    // a chip's picker can drop past its foot.
    let phone = phone();
    let scrim = if phone {
        format!(
            "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {}; display: flex; \
             background: {};",
            architect_shell::layer::SHEET,
            t.sheet
        )
    } else {
        format!(
            "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: {}; \
             background: {}; display: flex; align-items: flex-start; justify-content: center; \
             padding: 10vh 24px 24px; box-sizing: border-box;",
            architect_shell::layer::SHEET,
            t.scrim
        )
    };
    let card = if phone {
        format!(
            "display: flex; flex: 1; flex-direction: column; min-height: 0; min-width: 0; width: 100%; \
             background: {}; padding-bottom: {}px;",
            t.sheet,
            architect_shell::keyboard().height
        )
    } else {
        format!(
            "width: 672px; max-width: 100%; max-height: 80vh; display: flex; flex-direction: column; \
             border-radius: 12px; background: {}; border: 1px solid {}; box-shadow: {};",
            t.sheet, t.rule, t.shadow_float
        )
    };
    let mut c = capture;
    rsx! {
        if let Some((text, target)) = sheet {
            div { style: "{scrim}", onclick: move |_| c.sheet.set(None),
                div { style: "{card}", onclick: move |e| e.stop_propagation(),
                    Composer {
                        capture,
                        text,
                        target,
                        source: "ui",
                        on_close: move |()| c.sheet.set(None),
                    }
                }
            }
        }
        if deck {
            div { style: "{scrim}", onclick: move |_| c.deck.set(false),
                div { style: "{card}", onclick: move |e| e.stop_propagation(),
                    InboxDeck { capture, on_close: move |()| c.deck.set(false) }
                }
            }
        }
        if let Some(text) = window {
            if os.is_some() {
                CaptureWindow { capture, text }
            } else {
                div { style: "{scrim}", onclick: move |_| c.window.set(None),
                    div { style: "{card}", onclick: move |e| e.stop_propagation(),
                        Composer {
                            capture,
                            text,
                            target: None,
                            source: "window",
                            on_close: move |()| c.window.set(None),
                        }
                    }
                }
            }
        }
    }
}

/// The capture window: opened through the platform's windows when it
/// mounts, closed when it unmounts; the person closing it closes it here.
#[component]
fn CaptureWindow(capture: CaptureUi, text: String) -> Element {
    let os = try_consume_context::<architect_workspace::Windows>();
    let id = architect_workspace::WindowId(WINDOW.to_owned());
    // What the composer reads, carried into the window's own dom: the
    // shell and the theme (followed live), and this handle.
    let carried = use_hook(|| {
        architect_workspace::Bridge::new()
            .context::<Signal<architect_shell::Shell>>()
            .context::<architect_theme::ThemeHandle>()
            .context::<architect_theme::RootCss>()
            .with(Some(capture))
    });
    {
        let (id, os) = (id.clone(), os.clone());
        use_hook(move || {
            let Some(os) = os else { return };
            let draw: Rc<dyn Fn() -> Element> = Rc::new(move || {
                rsx! { WindowPage { capture, text: text.clone() } }
            });
            let gone = capture.window;
            os.0.open(architect_workspace::Popout::page(
                id,
                "Capture",
                None,
                carried,
                draw,
                Rc::new(move || {
                    let mut gone = gone;
                    gone.set(None);
                }),
            ));
        });
    }
    use_drop(move || {
        if let Some(os) = os {
            os.0.close(&id);
        }
    });
    rsx! {}
}

/// The capture window's document: the theme's ground, the composer
/// filling it.
#[component]
fn WindowPage(capture: CaptureUi, text: String) -> Element {
    let t = theme();
    let root = architect_theme::try_use_root_css().map(|r| r());
    #[cfg(feature = "native")]
    crate::shortcut::use_float_window();
    let ground = format!(
        "html, body {{ margin: 0; padding: 0; height: 100%; background: {}; }}",
        t.sheet
    );
    let mut c = capture;
    rsx! {
        document::Style { {ground} }
        if let Some(css) = root {
            document::Style { {css} }
        }
        div {
            style: "position: relative; width: 100vw; height: 100vh; display: flex; flex-direction: column; \
                    overflow: hidden; background: {t.sheet}; color: {t.ink}; font-family: {t.font}; font-size: 13px; \
                    padding-top: 22px; box-sizing: border-box;",
            Composer {
                capture,
                text,
                target: None,
                source: "window",
                on_close: move |()| c.window.set(None),
            }
        }
    }
}

/// A chip: what a property says, short.
fn chip_label(key: &str, value: &Value) -> Vec<String> {
    let text = |v: &Value| v.as_str().map_or_else(|| v.to_string(), str::to_owned);
    let unlink = |s: &str| {
        s.trim_start_matches("[[")
            .trim_end_matches("]]")
            .rsplit('/')
            .next()
            .unwrap_or(s)
            .trim_end_matches(".md")
            .to_owned()
    };
    let each = |v: &Value| -> Vec<String> {
        match v {
            Value::Array(a) => a.iter().map(text).collect(),
            other => vec![text(other)],
        }
    };
    match key {
        k::PROJECTS => each(value)
            .iter()
            .map(|p| format!("▸ {}", unlink(p)))
            .collect(),
        k::TAGS => each(value).iter().map(|t| format!("#{t}")).collect(),
        k::CONTEXTS => each(value),
        k::PRIORITY => vec![format!("! {}", text(value))],
        k::DUE => vec![format!("Due {}", day_label(&text(value)))],
        k::RECURRENCE => vec![format!("Repeats {}", repeat_label(&text(value)))],
        // The note's name says it already.
        k::RECURRENCE_ANCHOR | "title" => Vec::new(),
        other => vec![format!("{other}: {}", text(value))],
    }
}

/// `2026-11-06` as "Fri 6 Nov" (as it is when it is no date).
fn day_label(ymd: &str) -> String {
    chrono::NaiveDate::parse_from_str(ymd, "%Y-%m-%d")
        .map_or_else(|_| ymd.to_owned(), |d| d.format("%a %-d %b").to_string())
}

/// An RRULE in words, as far as quick add writes them.
pub(super) fn repeat_label(rule: &str) -> String {
    let part = |key: &str| {
        rule.split(';')
            .find_map(|p| p.strip_prefix(key).and_then(|v| v.strip_prefix('=')))
            .unwrap_or_default()
    };
    let every = match part("FREQ") {
        "DAILY" => "day",
        "WEEKLY" => "week",
        "MONTHLY" => "month",
        "YEARLY" => "year",
        _ => return rule.to_owned(),
    };
    let n = part("INTERVAL");
    let days = part("BYDAY");
    let base = if n.is_empty() || n == "1" {
        format!("every {every}")
    } else {
        format!("every {n} {every}s")
    };
    if days.is_empty() {
        base
    } else if days == "MO,TU,WE,TH,FR" {
        "every weekday".to_owned()
    } else {
        let names: Vec<&str> = days
            .split(',')
            .map(|d| match d {
                "MO" => "Monday",
                "TU" => "Tuesday",
                "WE" => "Wednesday",
                "TH" => "Thursday",
                "FR" => "Friday",
                "SA" => "Saturday",
                "SU" => "Sunday",
                other => other,
            })
            .collect();
        if n.is_empty() || n == "1" {
            format!("every {}", names.join(", "))
        } else {
            format!("every {n} weeks on {}", names.join(", "))
        }
    }
}

/// A JSON object as an action's facet value.
pub(super) fn facet_of(map: &Map<String, Value>) -> Option<facet_value::Value> {
    facet_json::from_str::<facet_value::Value>(&Value::Object(map.clone()).to_string()).ok()
}

/// What the deck does with a capture.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Deal {
    Into(String),
    Later(i64),
    Discard,
}

/// The inbox deck: the captures waiting, one at a time, each to make
/// into a target's thing, file, put off or discard.
#[component]
pub fn InboxDeck(capture: CaptureUi, on_close: EventHandler<()>) -> Element {
    let t = theme();
    let mut said = use_signal(|| None::<String>);
    let mut at = use_signal(|| 0_usize);
    // Read again after a change here, and every few seconds while it shows
    // (an agent's capture, an edit in the note).
    let mut tick = use_signal(|| 0_u64);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_secs(3)).await;
            let n = *tick.peek();
            tick.set(n.wrapping_add(1));
        }
    });
    let items = use_resource(move || {
        let _ = (capture.revision(), tick());
        let cap = capture.capturer();
        async move { cap.inbox(&ActionCaller::app(), false).await }
    });
    let targets = use_resource(move || {
        let cap = capture.capturer();
        async move { cap.targets(&ActionCaller::app()).await }
    });
    let (list, failed): (Vec<Item>, Option<String>) = match &*items.read() {
        Some(Ok(v)) => (v.clone(), None),
        Some(Err(e)) => (Vec::new(), Some(e.clone())),
        None => (Vec::new(), None),
    };
    let all = targets
        .read()
        .clone()
        .unwrap_or_else(|| vec![Target::note()]);
    let n = list.len();
    let i = (*at.read()).min(n.saturating_sub(1));
    let current = list.get(i).cloned();
    let act = {
        let current = current.clone();
        move |what: Deal| {
            let Some(item) = current.clone() else { return };
            let cap = capture.capturer();
            spawn(async move {
                let app = ActionCaller::app();
                let out = match &what {
                    Deal::Into(id) => cap.process(&app, &item.path, id, None, &Map::new()).await,
                    Deal::Later(days) => {
                        let day = actions::day_after(*days).format("%Y-%m-%d").to_string();
                        cap.defer(&app, &item.path, Some(&day)).await
                    }
                    Deal::Discard => cap.discard(&app, &item.path).await,
                };
                match out {
                    Ok(v) => {
                        let path = v.get("path").and_then(Value::as_str).unwrap_or_default();
                        said.set(Some(match what {
                            Deal::Into(id) if id == target::NOTE => format!("Filed as {path}"),
                            Deal::Into(_) => format!("Made {path}"),
                            Deal::Later(_) => format!("Put “{}” off", item.label()),
                            Deal::Discard => format!("Discarded “{}”", item.label()),
                        }));
                        capture.changed();
                    }
                    Err(e) => said.set(Some(e)),
                }
            });
        }
    };
    let button = {
        let t = t.clone();
        move |primary: bool| {
            format!(
                "padding: 7px 12px; border-radius: 8px; font-size: 13px; cursor: pointer; border: 1px solid {}; \
                 background: {}; color: {};",
                if primary { &t.accent } else { &t.rule },
                if primary { &t.accent } else { &t.pill },
                if primary { &t.accent_ink } else { &t.ink },
            )
        }
    };
    let card = current.map(|item| {
        let chips: Vec<String> = item
            .properties
            .iter()
            .flat_map(|(key, v)| chip_label(key, v))
            .collect();
        let when = item
            .created
            .get(..16)
            .unwrap_or(&item.created)
            .replace('T', " ");
        (item, chips, when)
    });
    rsx! {
        div {
            style: "display: flex; flex-direction: column; gap: 12px; padding: 16px; min-height: 0;",
            tabindex: 0,
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    on_close.call(());
                }
            },
            div { style: "display: flex; align-items: center; gap: 8px;",
                span { style: "font-size: 15px; font-weight: 600;", "Inbox" }
                span { style: "font-size: 12px; color: {t.ink_3};",
                    if n == 0 { "nothing waiting" } else { "{i + 1} of {n}" }
                }
                span { style: "flex: 1;" }
                if n > 1 {
                    button { style: "{button(false)}", onclick: move |_| at.set(i.saturating_sub(1)), "‹" }
                    button { style: "{button(false)}", onclick: move |_| at.set((i + 1).min(n - 1)), "›" }
                }
                button { style: "{button(false)}", onclick: move |_| on_close.call(()), "Done" }
            }
            if let Some(e) = failed {
                div { style: "font-size: 13px; color: {t.danger};", "{e}" }
            }
            if let Some((item, chips, when)) = card {
                div {
                    style: "display: flex; flex-direction: column; gap: 8px; padding: 14px; border-radius: 12px; \
                            border: 1px solid {t.rule}; background: {t.sheet};",
                    span { id: "inbox-title", style: "font-size: 16px; font-weight: 600;", "{item.label()}" }
                    if !item.body.is_empty() {
                        span { style: "font-size: 13px; color: {t.ink_2}; white-space: pre-wrap;", "{item.body}" }
                    }
                    if !chips.is_empty() {
                        div { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                            for (j, c) in chips.into_iter().enumerate() {
                                span {
                                    key: "{j}",
                                    style: "padding: 3px 8px; border-radius: 6px; font-size: 12px; background: {t.accent_soft};",
                                    "{c}"
                                }
                            }
                        }
                    }
                    span { style: "font-size: 11px; color: {t.ink_3};",
                        "{when}"
                        if !item.source.is_empty() { " · via {item.source}" }
                    }
                }
                div { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                    for tg in all.iter().cloned() {
                        button {
                            key: "{tg.id}",
                            id: "inbox-into-{tg.id}",
                            style: "{button(tg.id != target::NOTE)}",
                            title: "{tg.description}",
                            onclick: {
                                let act = act.clone();
                                move |_| act(Deal::Into(tg.id.clone()))
                            },
                            if tg.id == target::NOTE { "File as note" } else { "Make a {tg.name.to_lowercase()}" }
                        }
                    }
                    span { style: "flex: 1;" }
                    button {
                        style: "{button(false)}",
                        onclick: {
                            let act = act.clone();
                            move |_| act(Deal::Later(1))
                        },
                        "Tomorrow"
                    }
                    button {
                        style: "{button(false)}",
                        onclick: {
                            let act = act.clone();
                            move |_| act(Deal::Later(7))
                        },
                        "Next week"
                    }
                    button {
                        style: "{button(false)} color: {t.danger};",
                        onclick: move |_| act(Deal::Discard),
                        "Discard"
                    }
                }
            } else {
                div { style: "padding: 24px 0; text-align: center; color: {t.ink_2}; font-size: 13px;",
                    "Inbox zero. Capture with ⇧⌘N, or the capture window's shortcut from any app."
                }
            }
            if let Some(s) = said.read().clone() {
                div { style: "font-size: 12px; color: {t.ink_2};", "{s}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn chips_say_it_short() {
        assert_eq!(chip_label("projects", &json!(["[[Lab]]"])), ["▸ Lab"]);
        assert_eq!(chip_label("tags", &json!(["a", "b"])), ["#a", "#b"]);
        assert_eq!(chip_label("due", &json!("2026-11-06")), ["Due Fri 6 Nov"]);
        assert_eq!(
            chip_label("recurrence", &json!("FREQ=WEEKLY;BYDAY=MO")),
            ["Repeats every Monday"]
        );
        assert_eq!(
            chip_label("recurrence", &json!("FREQ=DAILY;INTERVAL=3")),
            ["Repeats every 3 days"]
        );
        assert!(chip_label("recurrence_anchor", &json!("scheduled")).is_empty());
        assert_eq!(chip_label("key", &json!("D")), ["key: D"]);
        assert!(chip_label("title", &json!("Gift idea")).is_empty());
    }
}
