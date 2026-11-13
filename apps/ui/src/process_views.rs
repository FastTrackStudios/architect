//! The Vault's "Process views" screenset (⌘6, feature `process-views`,
//! Linux): the same scene twice, side by side, to measure one against
//! the other —
//!
//! - **Process view**: `architect_process_view::demo::Orbits` served by
//!   the `process-view-demo` program, in a process of its own, its
//!   frames shared with the window's GPU (a dma-buf texture, or shared
//!   memory where the window can't import one);
//! - **Linked**: the same `Orbits` drawn in-process by the paint kind.
//!
//! Kill the program (its pid is on the pane's line) and the pane says
//! "View stopped" with a Restart button; drag the pane to another tab
//! group or a split and the program carries on (its session is kept by
//! the pane's id). `LAB_PROCESS_VIEW_STATE` answers with both panes'
//! numbers. The program is `ARCHITECT_LAB_PROCESS_VIEW`, else
//! `process-view-demo` beside the lab's executable
//! (`cargo build --bin process-view-demo --features architect-process-view/guest,architect-process-view/demo`).

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use architect::action::{ActionList, ActionMeta, Effect};
use architect_process_view::demo::{Orbits, Probe};
use architect_process_view::host::{ProcessViewPane, Program, Snapshot, Status, sessions};
use architect_process_view::wire::Transport;
use architect_workspace::{
    Extras, FrameStats, FrameSummary, PAINT, PaintMount, PaintPane, Pane, Side, Target, View,
    Workspace, use_pane,
};
use dioxus::prelude::*;

pub const PROCESS_PANE: &str = "process-view";
pub const LINKED_PANE: &str = "process-linked";

/// The program the process view runs.
fn program() -> Program {
    let path = std::env::var_os("ARCHITECT_LAB_PROCESS_VIEW")
        .map(PathBuf::from)
        .or_else(|| {
            Some(
                std::env::current_exe()
                    .ok()?
                    .parent()?
                    .join("process-view-demo"),
            )
        })
        .unwrap_or_else(|| PathBuf::from("process-view-demo"));
    let program = Program::new(path);
    match std::env::var("ARCHITECT_PROCESS_VIEW_TRANSPORT") {
        Ok(t) => program.env("ARCHITECT_PROCESS_VIEW_TRANSPORT", t),
        Err(_) => program,
    }
}

/// The linked pane's frame times and its input-to-frame times.
fn linked() -> &'static (FrameStats, Probe) {
    static LINKED: OnceLock<(FrameStats, Probe)> = OnceLock::new();
    LINKED.get_or_init(|| (FrameStats::new(), Probe::default()))
}

#[must_use]
pub fn views() -> Vec<View> {
    vec![
        View::new(
            "embed.process",
            "Process view",
            |_| rsx! { ProcessLabPane {} },
        )
        .drawn_by(architect_process_view::PROCESS),
        View::new(
            "embed.process-linked",
            "Linked",
            |_| rsx! { LinkedLabPane {} },
        )
        .drawn_by(PAINT),
    ]
}

/// The process view on the left, the same scene linked on the right.
#[must_use]
pub fn workspace() -> Workspace {
    let mut w = Workspace::new();
    let _ = w.open(
        Pane::new(PROCESS_PANE, "embed.process", "Process view"),
        Target::Tab,
    );
    let _ = w.open(
        Pane::new(LINKED_PANE, "embed.process-linked", "Linked"),
        Target::Split(Side::Right),
    );
    w
}

#[component]
fn ProcessLabPane() -> Element {
    let pane = use_pane();
    let program = use_hook(program);
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
            crate::embedded::KindHead {
                name: "Process view",
                kind: "process",
                about: "Drawn by process-view-demo, a program of its own: its GPU frames shared with this window. Point, press, use the arrows and type; kill it and restart it here.",
            }
            div { style: "flex: 1; display: flex; min-height: 0; margin: 4px 12px 8px;",
                ProcessViewPane { session_key: pane.id.0.clone(), program }
            }
        }
    }
}

#[component]
fn LinkedLabPane() -> Element {
    let mount = use_hook(|| {
        let (stats, probe) = linked();
        PaintMount::new(Orbits::new)
            .extras(Extras::default().with(probe.clone()))
            .stats(stats.clone())
    });
    let mut line = use_signal(linked_line);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(std::time::Duration::from_millis(200)).await;
            line.set(linked_line());
        }
    });
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
            crate::embedded::KindHead {
                name: "Linked",
                kind: "paint",
                about: "The same scene drawn in-process by the paint kind, for comparison.",
            }
            div { style: "flex: 1; position: relative; min-height: 0; margin: 4px 12px 8px; overflow: hidden;",
                div { style: "position: absolute; inset: 0;", PaintPane { mount } }
            }
            div { style: "padding: 0 12px 8px; font-size: 11px; color: #a1a1aa; font-family: monospace;",
                "{line}"
            }
        }
    }
}

fn ms(d: std::time::Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

fn linked_line() -> String {
    let (stats, probe) = linked();
    let (f, p) = (stats.summary(), probe.0.summary());
    format!(
        "{} frames · record p50 {:.2} / p99 {:.2} ms · input→frame p50 {:.2} / p99 {:.2} ms",
        f.frames,
        ms(f.p50),
        ms(f.p99),
        ms(p.p50),
        ms(p.p99)
    )
}

static STATE: ActionMeta = ActionMeta {
    id: "LAB_PROCESS_VIEW_STATE",
    trait_name: "",
    method_name: "",
    display_name: "The process views' numbers",
    description: "The Process views screenset (feature process-views): the process view's program (pid, running or stopped and why, how its frames are shared, their size), its frames, the program's frame time, input-to-frame and input-to-paint latency on the app's clock, and the widget's own paint time; and the same scene linked: its record times and input-to-frame latency. Times in microseconds (p50, p99, max).",
    category: "Lab",
    group: "Process views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static RESTART: ActionMeta = ActionMeta {
    id: "LAB_PROCESS_VIEW_RESTART",
    trait_name: "",
    method_name: "",
    display_name: "Restart the process view",
    description: "Stops the Process views screenset's program if it runs and starts it again (what the pane's Restart button does).",
    category: "Lab",
    group: "Process views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

fn us(s: FrameSummary) -> serde_json::Value {
    let u = |d: std::time::Duration| u64::try_from(d.as_micros()).unwrap_or(u64::MAX);
    serde_json::json!({ "n": s.frames, "p50": u(s.p50), "p99": u(s.p99), "max": u(s.max) })
}

fn snapshot_json(s: &Snapshot) -> serde_json::Value {
    let (status, why) = match &s.status {
        Status::Starting => ("starting", None),
        Status::Running => ("running", None),
        Status::Stopped(why) => ("stopped", Some(why.clone())),
    };
    serde_json::json!({
        "status": status,
        "why": why,
        "pid": s.pid,
        "starts": s.generation,
        "transport": s.transport.map(|t| match t { Transport::Dmabuf => "dmabuf", Transport::Shm => "shm" }),
        "size": s.size,
        "framesShown": s.frames,
        "guestFrame": us(s.render),
        "inputToFrame": us(s.input_to_frame),
        "inputToPaint": us(s.input_to_paint),
        "hostPaint": us(s.host_paint),
        "adapter": s.adapter,
    })
}

/// Puts `LAB_PROCESS_VIEW_STATE` and `LAB_PROCESS_VIEW_RESTART` on `list`.
pub fn register(list: &ActionList) {
    let _ = list.register_dynamic(
        &STATE,
        Effect::ReadOnly,
        None,
        None,
        Arc::new(|_: &[u8]| {
            let all = sessions()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .map(|(k, s)| (k.clone(), snapshot_json(&s.snapshot())))
                .collect::<serde_json::Map<_, _>>();
            let (stats, probe) = linked();
            let out = serde_json::json!({
                "process": all,
                "linked": { "record": us(stats.summary()), "inputToFrame": us(probe.0.summary()) },
            });
            Ok(serde_json::to_vec(&out).unwrap_or_default())
        }),
    );
    let _ = list.register_dynamic(
        &RESTART,
        Effect::Mutating,
        None,
        None,
        Arc::new(|_: &[u8]| {
            let all: Vec<_> = sessions()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .values()
                .cloned()
                .collect();
            for s in &all {
                s.restart();
            }
            Ok(b"{}".to_vec())
        }),
    );
}
