//! Keyflow in the lab's workspace: the source editor and the chart preview
//! as two panes over one parsed chart. Both follow the app's theme: the
//! editor through its `editor.*` tokens ([`EDITOR_TOKENS`]), the chart
//! through the card's colours until its toggle pins it.
use std::fmt::Write as _;
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
use keyflow::syntax::highlighting::{Color as HighlightColor, Theme as HighlightTheme};
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

/// The source editor alone, filling its pane edge to edge in the theme's
/// colours. Edits that parse update the shared chart; the preview pane
/// repaints it.
#[component]
pub fn KeyflowEditorPane() -> Element {
    let Some(shared) = shared() else {
        return rsx! { div { "Could not parse the sample chart" } };
    };
    let data = shared.data.clone();
    let wake = shared.mount.wake;
    // A typed slice: the pane restyles when these colours change (THEME_SET,
    // a theme file saved, a person's override), in a pop-out too.
    let colours = architect_theme::use_themed::<EditorColours>();
    let vars = colours.read().vars();
    let highlight = colours.read().highlight();
    rsx! {
        EditorStyles {}
        div { class: "lab-editor", style: "{vars}",
            KeyflowEditor {
                // The sheets come from EditorStyles: a `<style>` inside the
                // pane does not apply in Blitz, an app-level one does.
                stylesheet: false,
                initial: SOURCE.to_string(),
                id: Some(EDITOR_ID.to_string()),
                highlight_theme: highlight,
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
/// own CSS is): the editor-view sheet, the pane chrome, the syntax classes
/// and the lab's layout. The sheet never changes (a `document::Style` is
/// inserted once): every colour in it is a `var(…)` that
/// [`EditorColours::vars`] sets on the editor's root, so a theme change
/// restyles the editor by its `style` alone, in a pop-out too.
#[component]
fn EditorStyles() -> Element {
    // The highlight colours are CSS classes like the rest, so they need the
    // app-level sheet too: a `<style>` inside the pane does not apply in Blitz.
    let css = use_hook(|| {
        format!(
            "{EDITOR_CSS}\n{PANE_CSS}\n{}\n{EDITOR_LAYOUT}",
            highlight_sheet()
        )
    });
    rsx! { document::Style { {css} } }
}

/// The lab's layout over the editor's chrome: the editor fills its pane,
/// edge to edge, its text scrolling inside it. No pane header (the tab names
/// it), no rounded inset box: the pane's ground is the editor's. Colours
/// are only `var(…)`s, which [`EditorColours::vars`] sets from the theme.
const EDITOR_LAYOUT: &str = "
.lab-editor {
  box-sizing: border-box; width: 100%; height: 100%; min-width: 0; min-height: 0;
  display: flex; flex-direction: column;
  background: var(--code-bg); color: var(--foreground);
}
.lab-editor .kf-code-editor {
  display: flex; flex-direction: column; flex: 1 1 0; height: auto; min-height: 0;
}
.lab-editor .kf-pane-head { display: none; }
.lab-editor .kf-code-editor-pane {
  flex: 1 1 0; min-height: 0; border: 0; border-radius: 0; background: var(--code-bg);
}
.lab-editor .kf-code-editor-frame { box-sizing: border-box; height: auto; min-height: 100%; }
";

/// The editor's roles in the app's theme, with their defaults: expressions
/// over architect's tokens, so they follow whatever theme shows. The lab
/// sets them as its `App` layer ([`crate::theme`]); a theme file's
/// overrides or a person's say the last word. A role a theme breaks (or a
/// name missing) draws magenta, as every token does.
pub const EDITOR_TOKENS: &[(&str, &str)] = &[
    ("editor.background", "$background"),
    ("editor.foreground", "$foreground"),
    ("editor.gutter", "$muted-foreground"),
    ("editor.surface", "$muted"),
    ("editor.line", "$border"),
    ("editor.caret", "$brand"),
    ("editor.selection", "alpha($brand, 0.3)"),
    ("editor.syntax.chord", "$brand"),
    ("editor.syntax.quality", "mix($brand, $foreground, 0.45)"),
    ("editor.syntax.rhythm", "$warn"),
    ("editor.syntax.section", "$safe"),
    ("editor.syntax.meta", "$info"),
    ("editor.syntax.comment", "$muted-foreground"),
    ("editor.syntax.command", "mix($danger, $foreground, 0.3)"),
];

/// The syntax roles (`editor.syntax.<role>`), each the colour of a group of
/// Keyflow's highlight kinds ([`styled`]).
const SYNTAX: [&str; 7] = [
    "chord", "quality", "rhythm", "section", "meta", "comment", "command",
];

/// The editor's slice of the app's theme: its `editor.*` roles.
#[derive(Clone, PartialEq)]
struct EditorColours {
    background: architect_theme::Rgba,
    foreground: architect_theme::Rgba,
    gutter: architect_theme::Rgba,
    surface: architect_theme::Rgba,
    line: architect_theme::Rgba,
    caret: architect_theme::Rgba,
    selection: architect_theme::Rgba,
    /// By [`SYNTAX`].
    syntax: [architect_theme::Rgba; SYNTAX.len()],
}

impl architect_theme::FromResolved for EditorColours {
    fn from_resolved(r: &architect_theme::Resolved) -> Self {
        Self {
            background: r.color("editor.background"),
            foreground: r.color("editor.foreground"),
            gutter: r.color("editor.gutter"),
            surface: r.color("editor.surface"),
            line: r.color("editor.line"),
            caret: r.color("editor.caret"),
            selection: r.color("editor.selection"),
            syntax: SYNTAX.map(|role| r.color(&format!("editor.syntax.{role}"))),
        }
    }
}

impl EditorColours {
    /// The custom properties the editor's sheets read, on its root: editor-
    /// view's (`--background`, `--foreground`, `--muted`, …, and its
    /// `--editor-*` decoration hooks), the pane chrome's (`--code-bg`,
    /// `--ink`, `--line`, …) and the syntax classes' (`--editor-syntax-*`).
    /// Set on the editor's subtree only.
    fn vars(&self) -> String {
        let Self {
            background: bg,
            foreground: fg,
            gutter,
            surface,
            line,
            caret,
            selection,
            syntax,
        } = self;
        let mut vars = format!(
            "--background:{bg}; --foreground:{fg}; --muted:{surface}; \
             --muted-foreground:{gutter}; --border:{line}; --primary:{caret}; \
             --code-bg:{bg}; --ink:{fg}; --ink-soft:{gutter}; --line:{line}; \
             --editor-caret:{caret}; --editor-selection-bg:{selection}; \
             --editor-selection-line-bg:{selection}; --editor-preedit-underline:{caret};"
        );
        for (role, colour) in SYNTAX.iter().zip(syntax) {
            let _ = write!(vars, " --editor-syntax-{role}:{colour};");
        }
        vars
    }

    /// The syntax colours as Keyflow's highlight theme, for a host that
    /// applies the editor's own in-pane sheet (the web).
    fn highlight(&self) -> HighlightTheme {
        let c = |x: architect_theme::Rgba| {
            let [r, g, b, a] = x.packed().to_be_bytes();
            HighlightColor::rgba(r, g, b, a)
        };
        styled(c(self.background), c(self.foreground), |role| {
            c(self
                .syntax
                .get(role)
                .copied()
                .unwrap_or(architect_theme::Rgba::rgb(1.0, 0.0, 1.0)))
        })
    }
}

/// Keyflow's highlight theme with the default's weights and slants, its
/// ground and text `background` and `foreground`, and each highlight kind
/// the colour of its role (an index into [`SYNTAX`]).
fn styled(
    background: HighlightColor,
    foreground: HighlightColor,
    role: impl Fn(usize) -> HighlightColor,
) -> HighlightTheme {
    let mut t = HighlightTheme::default_dark();
    "Architect".clone_into(&mut t.name);
    t.background = background;
    t.foreground = foreground;
    t.unknown.color = foreground;
    let groups = [
        [
            &mut t.root,
            &mut t.scale_degree,
            &mut t.roman_numeral,
            &mut t.bass,
        ],
        [
            &mut t.accidental,
            &mut t.quality,
            &mut t.extension,
            &mut t.modifier,
        ],
        [
            &mut t.duration,
            &mut t.slash_rhythm,
            &mut t.rest,
            &mut t.push_pull,
        ],
        [
            &mut t.section,
            &mut t.measure_count,
            &mut t.section_comment,
            &mut t.measure_separator,
        ],
        [
            &mut t.title,
            &mut t.tempo,
            &mut t.time_signature,
            &mut t.key,
        ],
    ];
    for (i, group) in groups.into_iter().enumerate() {
        for style in group {
            style.color = role(i);
        }
    }
    t.comment.color = role(5);
    for style in [&mut t.command, &mut t.dynamic, &mut t.memory_recall] {
        style.color = role(6);
    }
    t
}

/// The syntax sheet with a `var(…)` for every colour: Keyflow's own
/// (`editor_keyflow::lang::highlight_css`) made with a stand-in colour per
/// role, each then swapped for its variable.
fn highlight_sheet() -> String {
    // `#0ed1NN`: no colour the sheet would hold of its own.
    let stand_in = |n: u8| HighlightColor::rgb(0x0e, 0xd1, n);
    let marks = u8::try_from(SYNTAX.len()).unwrap_or(u8::MAX);
    let mut css = editor_keyflow::lang::highlight_css(&styled(
        stand_in(marks),
        stand_in(marks.saturating_add(1)),
        |role| stand_in(u8::try_from(role).unwrap_or(u8::MAX)),
    ));
    let swaps = SYNTAX
        .iter()
        .map(|role| format!("var(--editor-syntax-{role})"))
        .chain(["var(--code-bg)".to_owned(), "var(--foreground)".to_owned()]);
    for (n, var) in (0u8..).zip(swaps) {
        css = css.replace(&format!("#0ed1{n:02x}"), &var);
    }
    css
}

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
                    apply(&chart, &command);
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
                PaintPane { mount: shared.mount.mount }
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

/// Set once `KEYFLOW_SET_CHART_THEME` has chosen the chart's theme: from then
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
fn apply(chart: &KeyflowChart, command: &ChartCommand) {
    match command {
        ChartCommand::View(view) => chart.data.update(|s| s.view = *view),
        ChartCommand::Dark(dark) => {
            CHART_PINNED.store(true, Ordering::Relaxed);
            *CHART_DARK.write() = *dark;
            let theme = if *dark {
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

/// A field of a request's arguments: `null` when it is missing, as indexing
/// the JSON gives.
fn arg<'a>(args: &'a serde_json::Value, key: &str) -> &'a serde_json::Value {
    static NULL: serde_json::Value = serde_json::Value::Null;
    args.get(key).unwrap_or(&NULL)
}

/// What the setters return.
fn ok() -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({ "ok": true })).unwrap_or_default()
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
            Ok(ok())
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
            let text = arg(&args, "text")
                .as_str()
                .ok_or_else(|| bad(id, "text: a string"))?;
            editor_keyflow::view::set_text(EDITOR_ID, text).map_err(|e| failed(id, e))?;
            Ok(ok())
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
            if let Some(offset) = arg(&args, "offset").as_u64() {
                let offset = usize::try_from(offset).unwrap_or(usize::MAX);
                editor_keyflow::view::set_cursor(EDITOR_ID, offset).map_err(|e| failed(id, e))?;
            } else {
                let line = arg(&args, "line")
                    .as_u64()
                    .ok_or_else(|| bad(id, "offset, or line (and col)"))?;
                let col = arg(&args, "col").as_u64().unwrap_or(0);
                let line = usize::try_from(line).unwrap_or(usize::MAX);
                let col = usize::try_from(col).unwrap_or(usize::MAX);
                editor_keyflow::view::set_cursor_at(EDITOR_ID, line, col)
                    .map_err(|e| failed(id, e))?;
            }
            Ok(ok())
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
            let zoom = match arg(&args, "zoom") {
                serde_json::Value::Null => None,
                zoom => Some(
                    zoom.as_f64()
                        .ok_or_else(|| bad(id, "zoom: a number or null"))?,
                ),
            };
            let scroll = (
                arg(&args, "scrollX")
                    .as_f64()
                    .ok_or_else(|| bad(id, "scrollX: a number"))?,
                arg(&args, "scrollY")
                    .as_f64()
                    .ok_or_else(|| bad(id, "scrollY: a number"))?,
            );
            queue(ChartCommand::View(ChartView {
                // The range the chart clamps a zoom to, so the state reports
                // the zoom it draws at.
                zoom: zoom.map(|zoom: f64| zoom.clamp(0.25, 4.0)),
                scroll,
            }));
            Ok(ok())
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
            let dark = arg(&args, "dark")
                .as_bool()
                .ok_or_else(|| bad(id, "dark: a boolean"))?;
            queue(ChartCommand::Dark(dark));
            Ok(ok())
        }),
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::cell::RefCell;
    use std::rc::Rc;

    use architect_theme::{LayerKind, ThemeStore};
    use blitz_dom::{BaseDocument, Document as _, DocumentConfig, NodeId};
    use blitz_traits::shell::{ColorScheme, Viewport};
    use dioxus_native::DioxusDocument;

    use super::*;

    thread_local! {
        static STORE: RefCell<Option<ThemeStore>> = const { RefCell::new(None) };
    }

    /// A head element asked for: its name, attributes and contents.
    type HeadElement = (String, Vec<(String, String)>, Option<String>);

    /// The document's head as dioxus-native's window gives it: what
    /// `document::Style` asks for goes into the document ([`settle`]).
    #[derive(Clone, Default)]
    struct Head(Rc<RefCell<Vec<HeadElement>>>);

    impl dioxus::document::Document for Head {
        fn eval(&self, js: String) -> dioxus::document::Eval {
            dioxus::document::NoOpDocument.eval(js)
        }

        fn create_head_element(
            &self,
            name: &str,
            attributes: &[(&str, String)],
            contents: Option<String>,
        ) {
            let attributes = attributes
                .iter()
                .map(|(k, v)| ((*k).to_owned(), v.clone()))
                .collect();
            self.0
                .borrow_mut()
                .push((name.to_owned(), attributes, contents));
        }
    }

    /// The editor's document and its head.
    struct App {
        doc: DioxusDocument,
        head: Head,
    }

    /// A 600×400 pane holding the editor, under a theme store with the
    /// lab's tokens and `theme` showing.
    fn app(theme: &'static str) -> App {
        let vdom = VirtualDom::new_with_props(
            |props: AppProps| {
                let handle = architect_theme::use_theme_store(ThemeStore::default);
                use_hook(|| {
                    let store = handle.store();
                    store.set(LayerKind::App, crate::theme::app_layer());
                    theme_set(&store, props.theme);
                    STORE.with(|s| *s.borrow_mut() = Some(store));
                });
                rsx! {
                    div { id: "pane", style: "position:absolute; left:0; top:0; width:600px; height:400px;",
                        KeyflowEditorPane {}
                    }
                }
            },
            AppProps { theme },
        );
        let head = Head::default();
        let document: Rc<dyn dioxus::document::Document> = Rc::new(head.clone());
        vdom.provide_root_context(document);
        let mut doc = DioxusDocument::new(
            vdom,
            DocumentConfig {
                viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
                ..Default::default()
            },
        );
        doc.initial_build();
        let mut app = App { doc, head };
        settle(&mut app);
        app
    }

    /// `THEME_SET {one: id}`.
    fn theme_set(store: &ThemeStore, id: &str) {
        let args = architect_theme::actions::SetArgs {
            one: Some(id.to_owned()),
            ..Default::default()
        };
        architect_theme::actions::set(store, &args).unwrap();
    }

    #[derive(Clone, PartialEq, Props)]
    struct AppProps {
        theme: &'static str,
    }

    /// Runs the app's turns and lays it out.
    fn settle(app: &mut App) {
        for _ in 0..8 {
            app.doc.poll(None);
            for (name, attributes, contents) in app.head.0.take() {
                app.doc.create_head_element(&name, &attributes, &contents);
            }
        }
        app.doc.inner.borrow_mut().resolve(0.0);
    }

    fn node(doc: &BaseDocument, selector: &str) -> NodeId {
        doc.query_selector(selector)
            .unwrap()
            .unwrap_or_else(|| panic!("no {selector}"))
    }

    /// `selector`'s box: where it is and how big.
    fn rect(doc: &BaseDocument, selector: &str) -> (f32, f32, f32, f32) {
        let n = doc.get_node(node(doc, selector)).unwrap();
        let at = n.absolute_position(0.0, 0.0);
        let size = n.final_layout().size;
        (at.x, at.y, size.width, size.height)
    }

    #[test]
    fn the_editor_fills_its_pane_with_no_colours_of_its_own() {
        let app = app("catppuccin-light");
        let doc = app.doc.inner.borrow();
        let pane = rect(&doc, "#pane");
        for part in [".lab-editor", ".kf-code-editor", ".kf-code-editor-pane"] {
            assert_eq!(rect(&doc, part), pane, "{part} fills the pane");
        }
        // No inset box: square corners, no border.
        let scroller = node(&doc, ".kf-code-editor-pane");
        for (property, none) in [
            ("border-top-left-radius", "0px"),
            ("border-top-width", "0px"),
        ] {
            assert_eq!(
                doc.resolved_style_value(scroller, property),
                none,
                "{property}"
            );
        }
        // Every colour on the editor's root is the theme's.
        let resolved = STORE.with(|s| s.borrow().as_ref().unwrap().current());
        let root = doc.get_node(node(&doc, ".lab-editor")).unwrap();
        let style = root
            .element_data()
            .and_then(|e| e.attr(blitz_dom::local_name!("style")))
            .unwrap_or_default()
            .to_owned();
        let expected =
            <EditorColours as architect_theme::FromResolved>::from_resolved(&resolved).vars();
        assert_eq!(style, expected);
        assert!(
            !style.contains("#202024"),
            "the old hard-coded dark: {style}"
        );
        assert!(
            resolved.misses().is_empty(),
            "unknown tokens: {:?}",
            resolved.misses()
        );
    }

    #[test]
    fn theme_set_restyles_the_editor() {
        let mut app = app("catppuccin-light");
        let background = |app: &App| {
            let d = app.doc.inner.borrow();
            let at = node(&d, ".kf-code-editor-pane");
            d.resolved_style_value(at, "background-color")
        };
        let light = background(&app);
        let store = STORE.with(|s| s.borrow().clone().unwrap());
        let want = |store: &ThemeStore| store.current().color("background");
        let light_bg = want(&store);
        assert!(light_bg.is_light(), "catppuccin-light's ground is light");

        theme_set(&store, "gruvbox-dark");
        settle(&mut app);
        let dark = background(&app);
        let dark_bg = want(&store);
        assert!(!dark_bg.is_light(), "gruvbox-dark's ground is dark");
        assert_ne!(light, dark, "the editor's ground follows THEME_SET");

        // A theme that breaks a role draws it magenta, not a fallback.
        store.set(
            LayerKind::Device,
            architect_theme::Layer::default().token("editor.background", "$nope"),
        );
        settle(&mut app);
        let broken = background(&app);
        assert!(
            broken.contains("255, 0, 255") || broken.contains("#ff00ff"),
            "a broken role shows magenta: {broken}"
        );
    }

    #[test]
    fn the_syntax_sheet_reads_every_role_and_no_colour_of_its_own() {
        let css = highlight_sheet();
        assert!(!css.contains("#0ed1"), "a stand-in colour left: {css}");
        for role in SYNTAX {
            assert!(
                css.contains(&format!("var(--editor-syntax-{role})")),
                "{role}"
            );
        }
        for (name, _) in EDITOR_TOKENS {
            assert!(name.starts_with("editor."), "{name}");
        }
    }
}
