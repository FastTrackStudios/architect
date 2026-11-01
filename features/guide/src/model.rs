//! A scenario as data: what [`crate::load`] reads out of a `.styx` file.
//!
//! Pure facet types, no Dioxus and no I/O, so a browser build (a web
//! tour) reads the same file the headless runner does.

use facet::Facet;
use facet_value::Value;

/// One scenario: a list of steps over an app, and how to film it.
///
/// ```styx
/// id palette-run
/// title "Run a command from ⌘K"
/// view {width 1280, height 800}
/// capture {fps 20, crop @id"palette", width 960}
/// steps (
///     {
///         id open
///         text "⌘K opens every command."
///         do @run{id LAB_SHOW_COMMANDS}
///         done {check ({id COMMANDS_STATE, has {overlay palette}})}
///     }
/// )
/// ```
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Scenario {
    /// Its id: the media's file names (`<id>.gif`, `<id>.png`), the
    /// test's name, what a guide refers to it by.
    pub id: String,
    /// What it shows, for a person: the tour's title.
    #[facet(default)]
    pub title: String,
    /// The window it runs in.
    #[facet(default)]
    pub view: ViewSize,
    /// How its frames become media.
    #[facet(default)]
    pub capture: Capture,
    /// Whether a tour may play it over a person's own data. Off: a tour
    /// only runs it in a playground the app sets up for it.
    #[facet(default)]
    pub safe: bool,
    /// What happens, in order.
    pub steps: Vec<Step>,
}

/// The main window: its content's size in CSS points, where that sits on
/// the desktop (`capture.desktop`), and its pixel density.
#[derive(Debug, Clone, Copy, PartialEq, Facet)]
pub struct ViewSize {
    pub width: u32,
    pub height: u32,
    /// The content's left edge on the desktop, points.
    #[facet(default)]
    pub x: f32,
    /// The content's top edge on the desktop, points (under its title
    /// bar, when the desktop draws one).
    #[facet(default)]
    pub y: f32,
    /// Device pixels per point (`2` for a retina capture).
    #[facet(default = 1.0)]
    pub scale: f32,
}

impl Default for ViewSize {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            x: 0.0,
            y: 0.0,
            scale: 1.0,
        }
    }
}

/// How a run's frames become media.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Capture {
    /// Frames a second, of the virtual clock and of the media.
    #[facet(default = 20)]
    pub fps: u32,
    /// What to keep of the window: a target, resolved at the first frame
    /// kept. Unset: the whole window.
    #[facet(default)]
    pub crop: Option<Target>,
    /// Points kept around the crop's target.
    #[facet(default = 16.0)]
    pub pad: f32,
    /// The media's width in pixels (the height follows). Unset: as
    /// rendered.
    #[facet(default)]
    pub width: Option<u32>,
    /// Which files to write. Empty: a GIF and a poster PNG.
    #[facet(default)]
    pub outputs: Vec<Output>,
    /// Draw the pointer and a ripple where it presses (the recording's
    /// own layer, never the app's).
    #[facet(default = true)]
    pub cursor: bool,
    /// Film a desktop of this size, every window on it (the main one at
    /// `view.x`/`view.y`, each pop-out where the workspace put it) with a
    /// title bar: what a drag out into a window of its own needs. Unset:
    /// the main window alone, edge to edge.
    #[facet(default)]
    pub desktop: Option<DesktopSize>,
}

/// A desktop's size, in points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
pub struct DesktopSize {
    pub width: u32,
    pub height: u32,
}

impl Default for Capture {
    fn default() -> Self {
        Self {
            fps: 20,
            crop: None,
            pad: 16.0,
            width: None,
            outputs: Vec::new(),
            cursor: true,
            desktop: None,
        }
    }
}

impl Capture {
    /// The outputs asked for, or a GIF and a poster when none are.
    #[must_use]
    pub fn outputs(&self) -> Vec<Output> {
        if self.outputs.is_empty() {
            vec![Output::Gif, Output::Png]
        } else {
            self.outputs.clone()
        }
    }
}

/// A file a capture writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Facet)]
#[repr(u8)]
#[facet(rename_all = "lowercase")]
pub enum Output {
    /// `<id>.gif`, through gifski.
    Gif,
    /// `<id>.webm` (VP9), through ffmpeg.
    Webm,
    /// `<id>.mp4` (H.264), through ffmpeg.
    Mp4,
    /// `<id>.png`: the last frame, a poster and the still.
    Png,
}

impl Output {
    /// The file's extension.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webm => "webm",
            Self::Mp4 => "mp4",
            Self::Png => "png",
        }
    }
}

/// One step: what is done, to what, and how to know it happened.
#[derive(Debug, Clone, PartialEq, Facet)]
pub struct Step {
    /// Its id, named when it fails.
    pub id: String,
    /// What the tour says while it waits for it.
    #[facet(default)]
    pub text: String,
    /// What it acts on (a gesture's start) and what a tour rings.
    #[facet(default)]
    pub target: Option<Target>,
    /// What is done. Unset: nothing (a step that only waits, or a tour's
    /// text-only step).
    #[facet(default, rename = "do")]
    pub act: Option<Do>,
    /// How to know it is done. Unset: done once `do` is.
    #[facet(default)]
    pub done: Option<Done>,
    /// Seconds the last frame is kept for, after it is done.
    #[facet(default)]
    pub hold: f32,
    /// Its frames are left out of the media (setup).
    #[facet(default)]
    pub hide: bool,
}

/// What a step does.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
#[facet(rename_all = "lowercase")]
pub enum Do {
    /// Runs an Action List action: `@run{id LAB_GO_VAULT, args {…}}`.
    Run {
        id: String,
        #[facet(default)]
        args: Option<Value>,
    },
    /// Presses and lets go on the step's target: `@click{}`.
    Click {
        /// Seconds the pointer takes to get there.
        #[facet(default = 0.4)]
        seconds: f32,
    },
    /// Presses on the step's target, moves, lets go. The end is `to`
    /// (its middle, or near its `edge`), else where it started, moved by
    /// `dx`/`dy` points: `@drag{to @group"files", edge right}` drops a tab
    /// on a group's right side; `@drag{dx 120}` pulls a divider along.
    Drag {
        #[facet(default)]
        to: Option<Target>,
        #[facet(default)]
        edge: Option<Edge>,
        #[facet(default)]
        dx: f32,
        #[facet(default)]
        dy: f32,
        #[facet(default = 0.8)]
        seconds: f32,
    },
    /// Turns the wheel over the step's target by `dx`/`dy` points, over
    /// `seconds`: `@scroll{dx 240}`.
    Scroll {
        #[facet(default)]
        dx: f32,
        #[facet(default)]
        dy: f32,
        #[facet(default = 0.6)]
        seconds: f32,
    },
    /// Types `text` into whatever has the focus, a key a frame:
    /// `@type{text "sched"}`.
    Type { text: String },
    /// Presses one key, with modifiers: `@key{key Enter, mods (meta)}`.
    /// A key is a `KeyboardEvent.key` name or one character.
    Key {
        key: String,
        #[facet(default)]
        mods: Vec<Modifier>,
    },
    /// Waits `seconds` of the virtual clock (an animation's length).
    Pause { seconds: f32 },
}

/// A side of a target, where a drag ends: a little way in from it
/// ([`Edge::point`]), as a drop on a group's side splits it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "lowercase")]
pub enum Edge {
    Left,
    Right,
    Top,
    Bottom,
}

impl Edge {
    /// How far in from the edge, as a share of the target's size…
    pub const INSET: f32 = 0.1;
    /// …and at most this many points: a drop zone along a side is a band
    /// of a few dozen points (a group's split zone; the window's dock band
    /// is the outer 28), wherever the target is large.
    pub const INSET_MOST: f32 = 40.0;

    /// The point a little way in from this edge of `(x, y, w, h)`,
    /// centred along it.
    #[must_use]
    pub fn point(self, x: f32, y: f32, w: f32, h: f32) -> (f32, f32) {
        let (cx, cy) = (w.mul_add(0.5, x), h.mul_add(0.5, y));
        let across = |size: f32| (size * Self::INSET).min(Self::INSET_MOST);
        match self {
            Self::Left => (x + across(w), cy),
            Self::Right => (x + w - across(w), cy),
            Self::Top => (cx, y + across(h)),
            Self::Bottom => (cx, y + h - across(h)),
        }
    }
}

/// A modifier key held through a [`Do::Key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "lowercase")]
pub enum Modifier {
    Shift,
    Ctrl,
    Alt,
    Meta,
}

/// Something on screen, as a query the runner (or a tour) resolves to a
/// rectangle at the frame it is needed: a layout change moves it.
#[derive(Debug, Clone, PartialEq, Facet)]
#[repr(u8)]
#[facet(rename_all = "kebab-case")]
pub enum Target {
    /// The element with this `id`.
    Id(String),
    /// The element with this `data-testid`.
    Testid(String),
    /// The first element a CSS selector matches.
    Css(String),
    /// A workspace tab, by its pane id (`architect-workspace`).
    Tab(String),
    /// A workspace pane's body, by its id.
    Pane(String),
    /// The workspace tab group holding this pane.
    Group(String),
    /// The tab bar of the group holding this pane: `@tabs"files"`. (Not
    /// `@tab-bar"…"`: styx reads `r"` as a raw string's start.)
    Tabs(String),
    /// The divider right after this pane (the innermost split's).
    Split(String),
    /// A row of the ⌘K palette, by its command's id
    /// (`architect-commands`).
    PaletteRow(String),
    /// The element whose whole text is this (trimmed), the last in the
    /// page (the innermost of those nested; a popover over the page): a
    /// menu's title or row, a settings section, a switcher's org.
    /// `@text"View"`.
    Text(String),
    /// A kind of target an app resolves itself: `@app{kind lane, name
    /// Drums}`.
    App { kind: String, name: String },
    /// A point, in window points.
    At { x: f32, y: f32 },
}

impl Target {
    /// Its kind and name, as an app's resolver is asked them: `tab` and
    /// a pane id. `None` for a point, which needs no resolving.
    #[must_use]
    pub fn kind_and_name(&self) -> Option<(&str, &str)> {
        Some(match self {
            Self::Id(n) => ("id", n),
            Self::Testid(n) => ("testid", n),
            Self::Css(n) => ("css", n),
            Self::Tab(n) => ("tab", n),
            Self::Pane(n) => ("pane", n),
            Self::Group(n) => ("group", n),
            Self::Tabs(n) => ("tab-bar", n),
            Self::Split(n) => ("split", n),
            Self::PaletteRow(n) => ("palette-row", n),
            Self::Text(n) => ("text", n),
            Self::App { kind, name } => (kind, name),
            Self::At { .. } => return None,
        })
    }
}

/// How to know a step is done: an event to wake on, and predicates.
#[derive(Debug, Clone, PartialEq, Default, Facet)]
pub struct Done {
    /// Action List events, any of which wakes the checks. Empty: the
    /// checks are tried on every frame.
    #[facet(default)]
    pub on: Vec<Wake>,
    /// Read-only actions with what their result must contain. All must
    /// hold.
    #[facet(default)]
    pub check: Vec<Check>,
    /// Seconds (of the wall clock) before the step fails.
    #[facet(default = 5.0)]
    pub timeout: f32,
}

/// An Action List event a step waits for.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
#[facet(rename_all = "lowercase")]
pub enum Wake {
    /// An action ran, and did not fail: `@ran"LAB_GO_VAULT"`.
    Ran(String),
    /// The app reported a change on this topic: `@changed"workspace"`.
    Changed(String),
}

/// A predicate: run read-only action `id` with `args`; its result must
/// contain `has` ([`crate::contains`]).
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Check {
    pub id: String,
    #[facet(default)]
    pub args: Option<Value>,
    pub has: Value,
}
