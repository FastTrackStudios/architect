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
//! and its own status bar in the shell's `status` slot. Its actions read
//! where they were invoked from: a pin from a note's context menu pins
//! that note.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use architect_extensions::linked::{Builder, Extension, Handle};
use dioxus::prelude::*;
use serde_json::{Value, json};

/// Its id.
pub const ID: &str = "lab.board";

/// Its status item's id.
const PINS: &str = "pins";

/// What is on the board, in this process.
#[derive(Debug, Default)]
struct State {
    pins: Vec<String>,
    chart: Option<String>,
    issues: Vec<String>,
    last: Value,
}

static STATE: Mutex<State> = Mutex::new(State {
    pins: Vec::new(),
    chart: None,
    issues: Vec::new(),
    last: Value::Null,
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
    let text = {
        let s = state();
        match s.pins.len() {
            0 => "Board ready: nothing pinned".to_owned(),
            1 => format!("1 pin: {}", s.pins[0]),
            n => format!("{n} pins, last {}", s.pins[n - 1]),
        }
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

/// The board, as `LAB_BOARD_STATE` says it.
fn snapshot() -> Value {
    let s = state();
    json!({
        "pins": s.pins,
        "chart": s.chart,
        "issues": s.issues,
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
        ext.view("board", BoardView)
            .view("chart", ChartView)
            .view("status-bar", BoardStatusBar)
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
#[component]
fn ChartView() -> Element {
    let _ = use_board();
    let chart = state().chart.clone();
    rsx! {
        div { style: "display: flex; flex-direction: column; gap: 8px; padding: 24px; color: var(--foreground);",
            div { style: "font-size: 16px; font-weight: 700;", "Keyflow chart" }
            div { style: "font-size: 13px; color: var(--muted-foreground);",
                {chart.map_or_else(|| "No chart opened yet.".to_owned(), |c| format!("Opened {c} with the Board extension."))}
            }
        }
    }
}

/// Its own status bar, in the shell's `status` slot.
#[component]
fn BoardStatusBar() -> Element {
    let _ = use_board();
    let (pins, chart) = {
        let s = state();
        (s.pins.len(), s.chart.clone())
    };
    rsx! {
        div { style: "height: 26px; flex-shrink: 0; box-sizing: border-box; display: flex; align-items: center; gap: 14px; \
                      padding: 0 12px; font-size: 12px; color: var(--primary-foreground); background: var(--primary);",
            span { style: "font-weight: 700;", "◆ Board's status bar" }
            span { "{pins} pinned" }
            if let Some(c) = chart {
                span { "chart: {c}" }
            }
            span { style: "flex: 1;" }
            span { "lab.board fills the status slot" }
        }
    }
}
