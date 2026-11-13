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
//! - the [`ProcessDeck`] (`INBOX_OPEN`): each capture once, a
//!   [`ProcessCard`] in Fast or Clarify mode, Task's ported (#445); and
//!   [`ProcessNowSheet`], one fresh capture processed on the spot;
//! - the capture window: the composer alone in a small OS window of its
//!   own, where the platform gives one (`architect_workspace::Popout::page`;
//!   feature `native` raises it over every app and puts it where the
//!   person is looking), else a card.

mod chips;
mod composer;
mod picker;
mod process;
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
pub use process::{
    ElapsedClock, LATER_DAYS, Mode, ModeSwitch, Origin, OriginTile, Outcome, ProcessCard,
    ProcessDeck, ProcessNowSheet, TaskShape, ago, context_suggestions, fmt_interval,
    rewritten_note, whole_text,
};
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
    /// A fresh capture being processed on the spot ("Process now").
    now: Signal<Option<Item>>,
    /// Bumped after every change made here, for lists to read again.
    revision: Signal<u64>,
    capturer: CopyValue<Capturer>,
}

impl CaptureUi {
    /// Opens the composer over the window, filled with `text`.
    pub fn compose(mut self, text: &str) {
        self.sheet.set(Some((text.to_owned(), None)));
        // Asked for: its field takes the keyboard as it opens, where the
        // keyboard is the screen's (on a tap alone otherwise).
        architect_shell::show_keyboard();
    }

    /// Opens the floating capture window (the global shortcut's).
    pub fn window(mut self, text: &str) {
        self.window.set(Some(text.to_owned()));
    }

    /// Opens the inbox deck.
    pub fn inbox(mut self) {
        self.deck.set(true);
    }

    /// Processes `item` on the spot, over the window ([`ProcessNowSheet`]).
    pub fn process(mut self, item: Item) {
        self.now.set(Some(item));
    }

    /// "Process now": captures `text` into the inbox, then processes it on
    /// the spot. Closing undecided leaves it there, as it was written.
    pub fn process_now(self, text: String) {
        let cap = self.capturer();
        spawn(async move {
            let app = ActionCaller::app();
            let args = actions::AddArgs {
                text,
                source: Some("ui".to_owned()),
                ..actions::AddArgs::default()
            };
            let Ok(v) = cap.capture(&app, args).await else {
                return;
            };
            self.changed();
            let path = v.get("path").and_then(Value::as_str).unwrap_or_default();
            if let Ok(item) = cap.item(&app, path).await {
                self.process(item);
            }
        });
    }

    /// Closes the composer over the window.
    pub fn close(mut self) {
        self.sheet.set(None);
    }

    /// Whether the composer, the deck or the capture window is open.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.sheet.read().is_some()
            || *self.deck.read()
            || self.window.read().is_some()
            || self.now.read().is_some()
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
    let mut now = use_signal(|| None::<Item>);
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
                    now.set(None);
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
        now,
        revision,
        capturer,
    };
    use_context_provider(|| ui)
}

/// How many captures wait to be processed, read again after every change
/// made here and every few seconds (an agent's capture): the phone's
/// capture button carries it. 0 with no capture.
#[must_use]
pub fn use_to_process(capture: Option<CaptureUi>) -> usize {
    let mut tick = use_signal(|| 0_u64);
    use_future(move || async move {
        if capture.is_none() {
            return;
        }
        loop {
            futures_timer::Delay::new(std::time::Duration::from_secs(10)).await;
            let n = *tick.peek();
            tick.set(n.wrapping_add(1));
        }
    });
    let count = use_resource(move || {
        let _ = (tick(), capture.map(|c| c.revision()));
        async move {
            match capture {
                Some(c) => c
                    .capturer()
                    .inbox(&ActionCaller::app(), false)
                    .await
                    .map_or(0, |v| v.len()),
                None => 0,
            }
        }
    });
    count.read().unwrap_or(0)
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
    let now = capture.now.read().clone();
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
    // The deck: Task's page, the window's ground, filling a phone, a
    // card a desk.
    let deck_card = if phone {
        format!(
            "display: flex; flex: 1; flex-direction: column; min-height: 0; min-width: 0; width: 100%; background: {};",
            t.desk
        )
    } else {
        format!(
            "width: 672px; max-width: 100%; height: 76vh; display: flex; flex-direction: column; overflow: hidden; \
             border-radius: 16px; background: {}; border: 1px solid {}; box-shadow: {};",
            t.desk, t.rule, t.shadow_float
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
                        on_process: move |text: String| capture.process_now(text),
                    }
                }
            }
        }
        if deck {
            div { style: "{scrim}", onclick: move |_| c.deck.set(false),
                div { style: "{deck_card}", onclick: move |e| e.stop_propagation(),
                    ProcessDeck { capture, on_exit: move |()| c.deck.set(false) }
                }
            }
        }
        if let Some(item) = now {
            ProcessNowSheet { capture, item, on_close: move |()| c.now.set(None) }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_repeat_in_words() {
        assert_eq!(repeat_label("FREQ=WEEKLY;BYDAY=MO"), "every Monday");
        assert_eq!(repeat_label("FREQ=DAILY;INTERVAL=3"), "every 3 days");
        assert_eq!(
            repeat_label("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"),
            "every weekday"
        );
    }
}
