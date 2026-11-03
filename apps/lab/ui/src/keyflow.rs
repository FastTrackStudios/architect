//! Keyflow in the lab's workspace: the source editor and the chart preview
//! as two panes over one parsed chart. The editor follows the app's dark
//! theme; the chart is white unless its toggle makes it dark.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect::action::{ActionError, ActionList, ActionMeta, Effect};
use architect_workspace::paint::PaintPane;
use architect_workspace::paint::peniko::Color;
use architect_workspace::{HostData, Pane, Side, Target, Workspace};
use dioxus::prelude::*;
use editor_keyflow::view::{KeyflowEditor, PANE_CSS, editor_view::EDITOR_CSS};
use facet::Facet;
use keyflow::engraver::renderer::ChartTheme;
use keyflow::syntax::highlighting::Theme as HighlightTheme;
use keyflow_ui::paint_view::{ChartMount, ChartSnapshot, ChartView, chart_mount};

const SOURCE: &str = "120bpm 4/4 #C\n\nvs\nC F G C\n";
/// The editor's id, which `editor_keyflow::view`'s `set_text`, `focus` and
/// `state` find it by.
const EDITOR_ID: &str = "keyflow-editor";

/// Whether the chart is dark. Only the chart's toggle sets it; the editor
/// does not follow it. App-lifetime: a signal made inside the shared context
/// would die with the pane that first asked for it.
static CHART_DARK: GlobalSignal<bool> = GlobalSignal::new(|| false);

/// What both panes share: the chart the editor writes and the preview
/// paints, and its mount.
#[derive(Clone)]
struct KeyflowChart {
    data: HostData<ChartSnapshot>,
    mount: ChartMount,
}

/// The one shared chart, made by whichever pane asks first.
fn shared() -> Option<KeyflowChart> {
    use_root_context(|| {
        keyflow::parse(SOURCE).ok().map(|chart| {
            let data = HostData::new(ChartSnapshot {
                chart,
                revision: 0,
                playhead_secs: None,
                theme: None,
                view: ChartView::default(),
            });
            let mount = chart_mount(data.clone());
            KeyflowChart { data, mount }
        })
    })
}

/// The source editor alone. Edits that parse update the shared chart; the
/// preview pane repaints it.
#[component]
pub fn KeyflowEditorPane() -> Element {
    let Some(shared) = shared() else {
        return rsx! { div { "Could not parse the sample chart" } };
    };
    let data = shared.data.clone();
    let wake = shared.mount.wake.clone();
    rsx! {
        EditorStyles {}
        div {
            style: "height:100%; min-height:0; min-width:0; {EDITOR_COLOURS}",
            KeyflowEditor {
                // The sheets come from EditorStyles: a `<style>` inside the
                // pane does not apply in Blitz, an app-level one does.
                stylesheet: false,
                initial: SOURCE.to_string(),
                id: Some(EDITOR_ID.to_string()),
                highlight_theme: HighlightTheme::default_dark(),
                on_change: move |source: String| {
                    if let Ok(parsed) = keyflow::parse(&source) {
                        data.update(|snapshot| {
                            snapshot.chart = parsed;
                            snapshot.revision = snapshot.revision.wrapping_add(1);
                        });
                        wake.wake();
                    }
                },
            }
        }
    }
}

/// The editor's stylesheets, at app level (`document::Style`, as the shell's
/// own CSS is): the editor-view sheet and the pane chrome.
#[component]
fn EditorStyles() -> Element {
    // The highlight colours are CSS classes like the rest, so they need the
    // app-level sheet too: a `<style>` inside the pane does not apply in Blitz.
    let highlight = editor_keyflow::lang::highlight_css(&HighlightTheme::default_dark());
    let css = format!("{EDITOR_CSS}\n{PANE_CSS}\n{highlight}\n{LAB_OVERRIDES}");
    rsx! { document::Style { {css} } }
}

/// The lab's own tweaks to the editor's chrome: no pane header (the tab
/// names it), and the caret in the lab's teal. The caret's shape is the
/// editor's own (a zero-width box with its bar hung off it, so the text
/// never moves); a host sets only its colour, through `--editor-caret`.
const LAB_OVERRIDES: &str =
    ".kf-pane-head { display: none; } .editor-root { --editor-caret: #3ec7a8; }";

/// The chart preview alone, with the theme toggle over it.
#[component]
pub fn KeyflowChartPane() -> Element {
    let Some(shared) = shared() else {
        return rsx! { div { "Could not parse the sample chart" } };
    };
    // The actions' changes are applied here, on the UI thread, once a frame.
    let loop_chart = shared.clone();
    use_future(move || {
        let chart = loop_chart.clone();
        async move {
            loop {
                futures_timer::Delay::new(COMMAND_POLL).await;
                for command in drain() {
                    apply(&chart, command);
                }
                publish(&chart);
            }
        }
    });
    // The chart follows the app's theme (a typed slice: it repaints only when
    // these colours change), until KEYFLOW_SET_CHART_THEME pins it.
    let colours = architect_theme::use_themed::<ChartColours>();
    let follow = shared.clone();
    use_effect(move || {
        let c = colours();
        if CHART_PINNED.load(Ordering::Relaxed) {
            return;
        }
        *CHART_DARK.write() = c.dark;
        follow.data.update(|s| s.theme = Some(c.theme()));
        follow.mount.wake.wake();
    });
    rsx! {
        div { style: "display:flex; flex-direction:column; height:100%; min-height:0;",
            div { style: "flex:1; min-height:0;",
                PaintPane { mount: shared.mount.mount.clone() }
            }
        }
    }
}

/// The "Chart" screenset: the source editor on the left, its preview on the right.
#[must_use]
pub fn chart_workspace() -> Workspace {
    let mut w = Workspace::new();
    let _ = w.open(
        Pane::new("chart-editor", "embed.keyflow-editor", "Editor"),
        Target::Tab,
    );
    let _ = w.open(
        Pane::new("chart-preview", "embed.keyflow-chart", "Chart"),
        Target::Split(Side::Right),
    );
    w
}

/// The editor's colours: the dark pane of the Architect theme. The chrome
/// and caret read these custom properties.
const EDITOR_COLOURS: &str = "--code-bg:#202024; --background:#202024; --foreground:#e9e9ee; --ink:#e9e9ee; --ink-soft:#9a9aa6; --accent:#3ec7a8;";

/// The chart's slice of the app's theme: its paper, its ink, its staff.
#[derive(Clone, PartialEq)]
struct ChartColours {
    dark: bool,
    paper: architect_theme::Rgba,
    ink: architect_theme::Rgba,
    staff: architect_theme::Rgba,
}

impl architect_theme::FromResolved for ChartColours {
    fn from_resolved(r: &architect_theme::Resolved) -> Self {
        Self {
            dark: r.scheme() == architect_theme::Scheme::Dark,
            paper: r.color("card"),
            ink: r.color("card-foreground"),
            staff: r.color("muted-foreground"),
        }
    }
}

impl ChartColours {
    fn theme(&self) -> ChartTheme {
        let c = |x: architect_theme::Rgba| Color::new([x.r, x.g, x.b, x.a]);
        let mut theme = ChartTheme::default();
        theme.render.background_color = c(self.paper);
        theme.render.foreground_color = c(self.ink);
        theme.render.staff_color = c(self.staff);
        theme
    }
}

/// Set once KEYFLOW_SET_CHART_THEME has chosen the chart's theme: from then
/// on it no longer follows the app's.
static CHART_PINNED: AtomicBool = AtomicBool::new(false);

fn dark_theme() -> ChartTheme {
    let mut theme = ChartTheme::default();
    theme.render.background_color = Color::from_rgb8(25, 30, 45);
    theme.render.foreground_color = Color::from_rgb8(245, 240, 220);
    theme.render.staff_color = Color::from_rgb8(210, 210, 205);
    theme
}

/// A chart change asked for from outside the UI thread, by a Keyflow action.
/// [`KeyflowChartPane`] applies it on the UI thread, where the chart's data
/// and the dark flag live.
pub enum ChartCommand {
    View(ChartView),
    Dark(bool),
}

/// The changes waiting for the chart pane to apply them.
static COMMANDS: Mutex<Vec<ChartCommand>> = Mutex::new(Vec::new());
/// The framing and theme as the chart pane last published them, which
/// `KEYFLOW_STATE` reads: the actions cannot reach the chart's data.
static PUBLISHED_VIEW: Mutex<ChartView> = Mutex::new(ChartView {
    zoom: None,
    scroll: (0.0, 0.0),
});
static PUBLISHED_DARK: AtomicBool = AtomicBool::new(false);

/// How often the chart pane applies what is waiting: about a frame.
const COMMAND_POLL: Duration = Duration::from_millis(16);

/// Queues `command` for the chart pane. Called from an action, so on the
/// control socket's thread.
fn queue(command: ChartCommand) {
    COMMANDS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(command);
}

/// Takes what is waiting, leaving the queue empty.
fn drain() -> Vec<ChartCommand> {
    std::mem::take(&mut *COMMANDS.lock().unwrap_or_else(PoisonError::into_inner))
}

/// Applies a waiting change on the UI thread, then asks for a frame.
fn apply(chart: &KeyflowChart, command: ChartCommand) {
    match command {
        ChartCommand::View(view) => chart.data.update(|s| s.view = view),
        ChartCommand::Dark(dark) => {
            CHART_PINNED.store(true, Ordering::Relaxed);
            *CHART_DARK.write() = dark;
            let theme = if dark {
                dark_theme()
            } else {
                ChartTheme::default()
            };
            chart.data.update(|s| s.theme = Some(theme));
        }
    }
    chart.mount.wake.wake();
}

/// Records what the chart shows now. The pane writes the wheel and the drag
/// into the view itself, so this reads the data, not only what was applied.
fn publish(chart: &KeyflowChart) {
    let view = chart.data.read(|s| s.view);
    *PUBLISHED_VIEW
        .lock()
        .unwrap_or_else(PoisonError::into_inner) = view;
    PUBLISHED_DARK.store(*CHART_DARK.peek(), Ordering::Relaxed);
}

fn published_view() -> ChartView {
    *PUBLISHED_VIEW
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// What `KEYFLOW_SET_SOURCE` takes.
#[derive(Debug, Clone, Facet)]
struct SourceArgs {
    text: String,
}

/// What `KEYFLOW_SET_CURSOR` takes: a byte `offset`, or a 0-based `line` and `col`.
#[derive(Debug, Clone, Facet)]
struct CursorArgs {
    offset: Option<u64>,
    line: Option<u64>,
    col: Option<u64>,
}

/// What `KEYFLOW_CHART_VIEW` takes.
#[derive(Debug, Clone, Facet)]
#[facet(rename_all = "camelCase")]
struct ChartViewArgs {
    /// `null` fits the page to the width.
    zoom: Option<f64>,
    scroll_x: f64,
    scroll_y: f64,
}

/// What `KEYFLOW_SET_CHART_THEME` takes.
#[derive(Debug, Clone, Facet)]
struct ChartThemeArgs {
    dark: bool,
}

static KEYFLOW_STATE: ActionMeta = ActionMeta {
    id: "KEYFLOW_STATE",
    trait_name: "",
    method_name: "",
    display_name: "Keyflow's state",
    description: "The Keyflow editor and chart. `editor` is its text, cursor (a byte offset) and focus, or null when no editor is mounted; `chart` is its theme (`dark`), zoom (`null` fits the page to the width) and scroll ([x, y], logical pixels), as the chart pane last published them.",
    category: "Lab",
    group: "Keyflow",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static KEYFLOW_FOCUS_EDITOR: ActionMeta = ActionMeta {
    id: "KEYFLOW_FOCUS_EDITOR",
    trait_name: "",
    method_name: "",
    display_name: "Focus Keyflow's editor",
    description: "Gives the Keyflow editor the keyboard focus.",
    category: "Lab",
    group: "Keyflow",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static KEYFLOW_SET_CURSOR: ActionMeta = ActionMeta {
    id: "KEYFLOW_SET_CURSOR",
    trait_name: "",
    method_name: "",
    display_name: "Set Keyflow's cursor",
    description: "Puts the caret in the Keyflow editor. Give `offset` (a byte offset into the text, clamped to its end), or `line` and `col` (both 0-based; `col` in characters, clamped to its line).",
    category: "Lab",
    group: "Keyflow",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static KEYFLOW_SET_SOURCE: ActionMeta = ActionMeta {
    id: "KEYFLOW_SET_SOURCE",
    trait_name: "",
    method_name: "",
    display_name: "Set Keyflow's source",
    description: "Replaces the whole text of the Keyflow editor with `text` and puts the cursor at its end. The chart follows, as it does for typing.",
    category: "Lab",
    group: "Keyflow",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static KEYFLOW_CHART_VIEW: ActionMeta = ActionMeta {
    id: "KEYFLOW_CHART_VIEW",
    trait_name: "",
    method_name: "",
    display_name: "Set the Keyflow chart's view",
    description: "Zooms and pans the Keyflow chart: `zoom` (1.0 is 100%, clamped to 0.25 to 4.0; `null` fits the page to the width), and `scrollX` / `scrollY` from the chart's top-left, in logical pixels. Applied on the chart pane's next frame.",
    category: "Lab",
    group: "Keyflow",
    shortcut: "",
    toggleable: false,
    undo: false,
};

static KEYFLOW_SET_CHART_THEME: ActionMeta = ActionMeta {
    id: "KEYFLOW_SET_CHART_THEME",
    trait_name: "",
    method_name: "",
    display_name: "Set the Keyflow chart's theme",
    description: "Sets the Keyflow chart to its dark theme (`dark: true`) or its light one. Applied on the chart pane's next frame.",
    category: "Lab",
    group: "Keyflow",
    shortcut: "",
    toggleable: false,
    undo: false,
};

fn failed(id: &str, message: String) -> ActionError {
    ActionError::Failed {
        id: id.to_owned(),
        message,
    }
}

fn bad(id: &str, message: impl Into<String>) -> ActionError {
    ActionError::BadArgs {
        id: id.to_owned(),
        message: message.into(),
        expected: None,
    }
}

/// An action's JSON arguments, decoded.
fn decode(id: &str, bytes: &[u8]) -> Result<serde_json::Value, ActionError> {
    serde_json::from_slice(bytes).map_err(|e| bad(id, e.to_string()))
}

/// What the setters return.
fn ok() -> Result<Vec<u8>, ActionError> {
    Ok(serde_json::to_vec(&serde_json::json!({ "ok": true })).unwrap_or_default())
}

/// Puts the Keyflow actions on `list`.
pub fn register(list: &ActionList) {
    let _ = list.register_dynamic(
        &KEYFLOW_STATE,
        Effect::ReadOnly,
        None,
        None,
        Arc::new(|_: &[u8]| {
            let editor = editor_keyflow::view::state(EDITOR_ID).map_or(
                serde_json::Value::Null,
                |s| serde_json::json!({ "text": s.text, "cursor": s.cursor, "focused": s.focused }),
            );
            let view = published_view();
            let out = serde_json::json!({
                "editor": editor,
                "chart": {
                    "dark": PUBLISHED_DARK.load(Ordering::Relaxed),
                    "zoom": view.zoom,
                    "scroll": [view.scroll.0, view.scroll.1],
                },
            });
            Ok(serde_json::to_vec(&out).unwrap_or_default())
        }),
    );
    let _ = list.register_dynamic(
        &KEYFLOW_FOCUS_EDITOR,
        Effect::Mutating,
        None,
        None,
        Arc::new(|_: &[u8]| {
            let id = KEYFLOW_FOCUS_EDITOR.id;
            editor_keyflow::view::focus(EDITOR_ID).map_err(|e| failed(id, e))?;
            ok()
        }),
    );
    let _ = list.register_dynamic(
        &KEYFLOW_SET_SOURCE,
        Effect::Mutating,
        Some(SourceArgs::SHAPE),
        None,
        Arc::new(|bytes: &[u8]| {
            let id = KEYFLOW_SET_SOURCE.id;
            let args = decode(id, bytes)?;
            let text = args["text"]
                .as_str()
                .ok_or_else(|| bad(id, "text: a string"))?;
            editor_keyflow::view::set_text(EDITOR_ID, text).map_err(|e| failed(id, e))?;
            ok()
        }),
    );
    let _ = list.register_dynamic(
        &KEYFLOW_SET_CURSOR,
        Effect::Mutating,
        Some(CursorArgs::SHAPE),
        None,
        Arc::new(|bytes: &[u8]| {
            let id = KEYFLOW_SET_CURSOR.id;
            let args = decode(id, bytes)?;
            if let Some(offset) = args["offset"].as_u64() {
                let offset = usize::try_from(offset).unwrap_or(usize::MAX);
                editor_keyflow::view::set_cursor(EDITOR_ID, offset).map_err(|e| failed(id, e))?;
            } else {
                let line = args["line"]
                    .as_u64()
                    .ok_or_else(|| bad(id, "offset, or line (and col)"))?;
                let col = args["col"].as_u64().unwrap_or(0);
                let line = usize::try_from(line).unwrap_or(usize::MAX);
                let col = usize::try_from(col).unwrap_or(usize::MAX);
                editor_keyflow::view::set_cursor_at(EDITOR_ID, line, col)
                    .map_err(|e| failed(id, e))?;
            }
            ok()
        }),
    );
    let _ = list.register_dynamic(
        &KEYFLOW_CHART_VIEW,
        Effect::Mutating,
        Some(ChartViewArgs::SHAPE),
        None,
        Arc::new(|bytes: &[u8]| {
            let id = KEYFLOW_CHART_VIEW.id;
            let args = decode(id, bytes)?;
            let zoom = match &args["zoom"] {
                serde_json::Value::Null => None,
                zoom => Some(
                    zoom.as_f64()
                        .ok_or_else(|| bad(id, "zoom: a number or null"))?,
                ),
            };
            let scroll = (
                args["scrollX"]
                    .as_f64()
                    .ok_or_else(|| bad(id, "scrollX: a number"))?,
                args["scrollY"]
                    .as_f64()
                    .ok_or_else(|| bad(id, "scrollY: a number"))?,
            );
            queue(ChartCommand::View(ChartView {
                // The range the chart clamps a zoom to, so the state reports
                // the zoom it draws at.
                zoom: zoom.map(|zoom: f64| zoom.clamp(0.25, 4.0)),
                scroll,
            }));
            ok()
        }),
    );
    let _ = list.register_dynamic(
        &KEYFLOW_SET_CHART_THEME,
        Effect::Mutating,
        Some(ChartThemeArgs::SHAPE),
        None,
        Arc::new(|bytes: &[u8]| {
            let id = KEYFLOW_SET_CHART_THEME.id;
            let args = decode(id, bytes)?;
            let dark = args["dark"]
                .as_bool()
                .ok_or_else(|| bad(id, "dark: a boolean"))?;
            queue(ChartCommand::Dark(dark));
            ok()
        }),
    );
}
