//! The lab's example extension (`apps/lab/board/extension.toml`): a pin
//! board. One [`Board`] value runs it two ways — the lab links it in
//! (`Extensions::with_linked`), and its program (`src/main.rs`) serves
//! the same value as a process (`Guest::extension`) — so a development
//! copy of `lab.board` runs out of process and overrides the linked one.
//!
//! What it adds: a tab (the board), a status item (what is pinned), a
//! "Pin to the board" entry in a note's context menu, an opener for `.kf`
//! files (and the `keyflow.chart` document kind), an opener for GitHub
//! issue links, a View-menu entry, a phone bar button, a startup hook,
//! its own status bar in the shell's `status` slot, and two inline
//! embeds: ```` ```kf ```` fences drawn as a chart by a page
//! (`pages/kf.tsx`), and `type: song` notes given a song card
//! ([`SongCard`]). Its actions read
//! where they were invoked from: a pin from a note's context menu pins
//! that note.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use architect_extensions::contrib::{StateKind, StateWrite};
use architect_extensions::linked::{Builder, Extension, Handle};
use dioxus::prelude::*;
use serde_json::{Value, json};

/// Its id.
pub const ID: &str = "lab.board";

/// Its page files (`pages/`): the tab's page and the worker it spawns,
/// for an app that runs pages to write out as the built-in's
/// (`architect_pages::files::write_builtin`).
pub const PAGES: &[(&str, &str)] = &[
    ("board.tsx", include_str!("../pages/board.tsx")),
    ("kf.tsx", include_str!("../pages/kf.tsx")),
    ("primes.js", include_str!("../pages/primes.js")),
    (
        "summary.a2ui.json",
        include_str!("../pages/summary.a2ui.json"),
    ),
    (
        "luau-primes.luau",
        include_str!("../pages/luau-primes.luau"),
    ),
    ("count.luau", include_str!("../pages/count.luau")),
];

/// Its status item's id.
const PINS: &str = "pins";

/// What is on the board, in this process.
#[derive(Debug, Default)]
struct State {
    pins: Vec<String>,
    chart: Option<String>,
    issues: Vec<String>,
    last: Value,
    /// The tempo widget's, beats a minute.
    tempo: u32,
    /// The chord sheet's line.
    chords: String,
    /// Sharps on for the chords put in next.
    sharp: bool,
}

static STATE: Mutex<State> = Mutex::new(State {
    pins: Vec::new(),
    chart: None,
    issues: Vec::new(),
    last: Value::Null,
    tempo: 120,
    chords: String::new(),
    sharp: false,
});

/// Moves on with every change, for the views.
static REVISION: AtomicU64 = AtomicU64::new(0);

/// Where its runtime state goes: the copy built last's.
static HANDLE: Mutex<Option<Handle>> = Mutex::new(None);

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The board changed: the status item says so, the views look again.
fn changed() {
    let pins = state().pins.clone();
    let text = match (pins.len(), pins.last()) {
        (_, None) => "Board ready: nothing pinned".to_owned(),
        (1, Some(only)) => format!("1 pin: {only}"),
        (n, Some(last)) => format!("{n} pins, last {last}"),
    };
    let handle = HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(h) = handle {
        h.status(PINS, text);
    }
    REVISION.fetch_add(1, Ordering::Relaxed);
}

/// The chords its Typing Toolbar puts in (`LAB_BOARD_CHORD_<name>`).
const CHORDS: [&str; 7] = ["C", "D", "E", "F", "G", "A", "B"];

/// The chord sheet, as `LAB_BOARD_CHORDS` says it.
fn chords() -> Value {
    let s = state();
    json!({ "text": s.chords, "sharp": s.sharp })
}

/// Puts `what` at the end of the chord sheet's line.
fn put(what: &str) -> Value {
    {
        let mut s = state();
        if !s.chords.is_empty() && !s.chords.ends_with(' ') {
            s.chords.push(' ');
        }
        s.chords.push_str(what);
    }
    changed();
    chords()
}

/// Sharps on or off: the toggle's state, for its button.
fn sharp(on: bool) -> Value {
    state().sharp = on;
    let handle = HANDLE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    if let Some(h) = handle {
        h.set_state(StateWrite {
            key: "LAB_BOARD_SHARP".to_owned(),
            kind: StateKind::Checked,
            text: if on { "on" } else { "off" }.to_owned(),
            ..StateWrite::default()
        });
    }
    changed();
    chords()
}

/// The board, as `LAB_BOARD_STATE` says it.
// Read whole under the lock: one consistent board.
#[allow(clippy::significant_drop_tightening)]
fn snapshot() -> Value {
    let s = state();
    json!({
        "pins": s.pins,
        "chart": s.chart,
        "issues": s.issues,
        "tempo": s.tempo,
        "last_invoked": s.last,
        "process": std::process::id(),
    })
}

fn pin(args: &Value, cx: Value) -> Value {
    let what = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| cx.get("id").and_then(Value::as_str).map(str::to_owned))
        .unwrap_or_else(|| "something".to_owned());
    {
        let mut s = state();
        s.pins.push(what.clone());
        s.last = cx;
    }
    changed();
    json!({ "pinned": what, "state": snapshot() })
}

/// The extension.
#[derive(Debug, Clone, Copy, Default)]
pub struct Board;

impl Extension for Board {
    fn manifest(&self) -> &'static str {
        include_str!("../extension.toml")
    }

    fn build(&self, ext: &mut Builder) {
        *HANDLE.lock().unwrap_or_else(PoisonError::into_inner) = Some(ext.handle());
        // A new copy's sharps start off: a toggle's state is the host's,
        // and goes with the copy before.
        state().sharp = false;
        ext.view("board", BoardView)
            .view("chart", ChartView)
            .view("song", SongCard)
            .view("tempo", TempoWidget)
            .view("summary", PagesOnly)
            .view("luau-primes", PagesOnly)
            .action_in(
                "LAB_BOARD_PIN",
                |args, cx| async move { Ok(pin(&args, cx)) },
            )
            .action("LAB_BOARD_CLEAR", |_| async {
                state().pins.clear();
                changed();
                Ok(snapshot())
            })
            .action("LAB_BOARD_STATE", |_| async { Ok(snapshot()) })
            .action("LAB_BOARD_CHORDS", |_| async { Ok(chords()) })
            .action("LAB_BOARD_SHARP", |_| async {
                let on = !state().sharp;
                Ok(sharp(on))
            })
            .action("LAB_BOARD_BAR_LINE", |_| async { Ok(put("|")) })
            .action("LAB_BOARD_CHORDS_CLEAR", |_| async {
                state().chords.clear();
                changed();
                Ok(chords())
            })
            .action_in("LAB_BOARD_HELLO", |_, cx| async move {
                state().last = cx;
                changed();
                Ok(json!("ready"))
            })
            .action_in("LAB_BOARD_OPENED", |_, cx| async move {
                {
                    let mut s = state();
                    s.chart = cx.get("detail").and_then(Value::as_str).map(str::to_owned);
                    s.last = cx;
                }
                changed();
                Ok(snapshot())
            })
            .action_in("LAB_BOARD_ISSUE", |_, cx| async move {
                let url = cx
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let number = url.rsplit('/').next().unwrap_or_default().to_owned();
                {
                    let mut s = state();
                    s.issues.push(url.clone());
                    s.pins.push(format!("issue #{number}"));
                    s.last = cx;
                }
                changed();
                Ok(json!({ "pinned": url }))
            });
        for name in CHORDS {
            ext.action(format!("LAB_BOARD_CHORD_{name}"), move |_| async move {
                let chord = if state().sharp {
                    format!("{name}#")
                } else {
                    name.to_owned()
                };
                Ok(put(&chord))
            });
        }
    }
}

/// Re-renders the calling view whenever the board changes.
fn use_board() -> u64 {
    let mut seen = use_signal(|| REVISION.load(Ordering::Relaxed));
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(Duration::from_millis(200)).await;
            let now = REVISION.load(Ordering::Relaxed);
            if *seen.peek() != now {
                seen.set(now);
            }
        }
    });
    seen()
}

/// The board's tab: what is pinned.
#[component]
fn BoardView() -> Element {
    let _ = use_board();
    let (pins, chart) = {
        let s = state();
        (s.pins.clone(), s.chart.clone())
    };
    let pid = std::process::id();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 10px; padding: 24px; \
                      color: var(--foreground); font-size: 13px;",
            div { style: "font-size: 18px; font-weight: 700;", "Board" }
            div { style: "color: var(--muted-foreground);",
                "The lab's example extension (lab.board), drawn in process {pid}. \
                 Right-click a note's tab and Pin to the board; open a .kf file or a GitHub issue link."
            }
            if pins.is_empty() {
                div { style: "color: var(--muted-foreground);", "Nothing pinned yet." }
            }
            for (i, p) in pins.into_iter().enumerate() {
                div { key: "{i}",
                    style: "padding: 8px 12px; border-radius: 8px; border: 1px solid var(--border); \
                            background: var(--card);",
                    "📌 {p}"
                }
            }
            if let Some(chart) = chart {
                div { style: "color: var(--muted-foreground);", "Last chart: {chart}" }
            }
        }
    }
}

/// The `.kf` opener's view.
/// What `summary` and `luau-primes` show in an app that does not run their
/// page files (`pages/summary.a2ui.json`, `pages/luau-primes.luau`).
#[component]
fn PagesOnly() -> Element {
    rsx! {
        div { style: "padding: 24px; font-size: 13px; color: var(--muted-foreground);",
            "This view is a page file: an app that runs pages (architect-pages) draws it."
        }
    }
}

#[component]
fn ChartView() -> Element {
    let _ = use_board();
    let (chart, text, on) = {
        let s = state();
        (s.chart.clone(), s.chords.clone(), s.sharp)
    };
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 8px; padding: 24px; color: var(--foreground);",
            div { style: "font-size: 16px; font-weight: 700;", "Keyflow chart" }
            div { style: "font-size: 13px; color: var(--muted-foreground);",
                {chart.map_or_else(|| "No chart opened yet.".to_owned(), |c| format!("Opened {c} with the Board extension."))}
            }
            // Its chord line: on a phone the Board's chord buttons ride
            // over the keyboard while it has the focus (the manifest's
            // `[[toolbars]] where = "typing"`).
            input {
                id: "chords-text",
                r#type: "text",
                "aria-label": "Chords",
                value: "{text}",
                style: "height: 44px; box-sizing: border-box; padding: 0 12px; border-radius: 10px; \
                        border: 1px solid var(--input); background: var(--background); color: var(--foreground); \
                        font-size: 16px;",
                oninput: move |e: FormEvent| {
                    state().chords = e.value();
                    changed();
                },
            }
            div { "data-testid": "chords-sharp", style: "font-size: 13px; color: var(--muted-foreground);",
                if on { "Sharps on" } else { "Sharps off" }
            }
        }
    }
}

/// The tempo, as a toolbar widget: its beats a minute between a step
/// down and a step up.
#[component]
fn TempoWidget() -> Element {
    let _ = use_board();
    let bpm = state().tempo;
    let nudge = |by: i32| {
        move |_| {
            {
                let mut s = state();
                s.tempo = s.tempo.saturating_add_signed(by).clamp(20, 300);
            }
            changed();
        }
    };
    let step = "width: 20px; height: 20px; border: none; border-radius: 5px; cursor: pointer; \
                background: var(--secondary); color: var(--secondary-foreground); font-size: 12px;";
    rsx! {
        div {
            "data-testid": "board-tempo",
            style: "flex: 1; display: flex; align-items: center; justify-content: center; gap: 6px; padding: 0 6px; \
                    font-size: 12px; color: var(--foreground); background: var(--muted); border-radius: 6px;",
            button { style: "{step}", "aria-label": "Slower", onclick: nudge(-1), "−" }
            span { style: "min-width: 52px; text-align: center; font-variant-numeric: tabular-nums;", "{bpm} BPM" }
            button { style: "{step}", "aria-label": "Faster", onclick: nudge(1), "+" }
        }
    }
}

/// A `type: song` note's card, as an inline embed at the note's head: its
/// frontmatter (title, artist, key, tempo) and how long its lyric is.
/// In a pane of its own (no embed), what it is for.
#[component]
pub fn SongCard() -> Element {
    let Some(input) = architect_extensions::linked::use_embed() else {
        return rsx! {
            div { style: "padding: 24px; font-size: 13px; color: var(--muted-foreground);",
                "The song card shows at the head of a note whose type is song."
            }
        };
    };
    let field = |name: &str| {
        input
            .props
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let (title, artist, key, tempo) = (
        field("title"),
        field("artist"),
        field("key"),
        field("tempo"),
    );
    // The lyric: its lines up to the first fenced block (a chart).
    let lines = input
        .source
        .lines()
        .take_while(|l| !l.trim_start().starts_with("```"))
        .filter(|l| !l.trim().is_empty())
        .count();
    let chips = [("Key", key), ("Tempo", tempo)]
        .into_iter()
        .filter(|(_, v)| !v.is_empty());
    rsx! {
        div { class: "song-card",
            style: "display: flex; flex-direction: column; gap: 8px; padding: 14px 16px; border-radius: 10px; \
                    border: 1px solid var(--border); background: var(--card); color: var(--card-foreground);",
            div { style: "display: flex; align-items: baseline; gap: 10px;",
                span { style: "font-size: 12px; font-weight: 700; letter-spacing: 0.06em; color: var(--primary);", "♪ SONG" }
                span { class: "song-title", style: "font-size: 17px; font-weight: 700;",
                    if title.is_empty() { "Untitled song" } else { "{title}" }
                }
                if !artist.is_empty() {
                    span { style: "font-size: 13px; color: var(--muted-foreground);", "{artist}" }
                }
            }
            div { style: "display: flex; flex-wrap: wrap; gap: 6px;",
                for (name, value) in chips {
                    span { key: "{name}",
                        style: "padding: 2px 8px; border-radius: 999px; font-size: 12px; \
                                background: var(--secondary); color: var(--secondary-foreground);",
                        "{name} {value}"
                    }
                }
                span { style: "padding: 2px 8px; font-size: 12px; color: var(--muted-foreground);",
                    {format!("{lines} line{} of lyric", if lines == 1 { "" } else { "s" })}
                }
            }
        }
    }
}
