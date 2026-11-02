//! The Embedded views' paint pane: the `paint` kind
//! (`architect_workspace::paint`), a 100 × 100 grid of rectangles drawn
//! in-process by Blitz each frame, its picture and its hit test one list
//! ([`Marks`]). The pointer lights the rectangle under it, a press picks
//! one, the arrow keys move the pick, and what is typed (composed text
//! too) is kept — in the pane's typed host data ([`HostData`]), which the
//! line under it reads and `LAB_PAINT_STATE` answers with, with the
//! pane's frame times.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use architect::action::{ActionList, ActionMeta, Effect};
use architect_shell::use_shell;
use architect_workspace::paint::peniko::kurbo::{Affine, Point, Rect};
use architect_workspace::paint::peniko::{Color, Fill};
use architect_workspace::paint::{PaintScene as _, Scene};
use architect_workspace::{
    Extras, FrameStats, HostData, Mark, Marks, PaintMount, PaintPane, PaintView, ViewCtx, ViewInput,
};
use dioxus::prelude::*;
use facet::Facet;

/// The grid's columns and rows.
pub const COLS: u32 = 100;
pub const ROWS: u32 = 100;
/// The most typed text kept.
const TYPED: usize = 64;

/// What the pane's view keeps for its host: the host's typed data.
#[derive(Debug, Clone, Default, PartialEq, Eq, Facet)]
pub struct PaintState {
    /// The rectangle under the pointer, by its id (`row * 100 + col`).
    pub hover: Option<u64>,
    /// The one pressed, or moved to with the arrow keys.
    pub picked: Option<u64>,
    /// What was typed or composed, the last [`TYPED`] characters.
    pub typed: String,
    /// What is being composed (an IME's preedit).
    pub preedit: String,
    pub focused: bool,
    /// Input events the view was given.
    pub events: u64,
}

/// The lab's one paint pane's data and frame times.
fn shared() -> &'static (HostData<PaintState>, FrameStats) {
    static SHARED: OnceLock<(HostData<PaintState>, FrameStats)> = OnceLock::new();
    SHARED.get_or_init(|| (HostData::new(PaintState::default()), FrameStats::new()))
}

/// The grid: laid out over the pane's size, drawn and hit-tested from
/// the same marks.
pub struct Grid {
    marks: Marks,
    laid: Option<(f32, f32)>,
    state: HostData<PaintState>,
    hover: Option<u64>,
    picked: Option<u64>,
    sheet: Color,
    lit: Color,
    pick: Color,
}

impl Grid {
    /// Its state in the host's [`PaintState`] (a fresh one without it).
    #[must_use]
    pub fn new(ctx: &ViewCtx) -> Self {
        let token = |name: &str, or: Color| ctx.theme.color(name).unwrap_or(or);
        Self {
            marks: Marks::new(),
            laid: None,
            state: ctx
                .data::<PaintState>()
                .unwrap_or_else(|| HostData::new(PaintState::default())),
            hover: None,
            picked: None,
            sheet: token("sheet", Color::from_rgba8(24, 24, 27, 255)),
            lit: token("accent", Color::from_rgba8(255, 122, 26, 255)),
            pick: token("ink", Color::from_rgba8(250, 250, 250, 255)),
        }
    }

    /// The cells over `width` × `height`, a pixel apart when there's room.
    fn lay_out(&mut self, width: f32, height: f32) {
        if self.laid == Some((width, height)) {
            return;
        }
        self.laid = Some((width, height));
        let (w, h) = (
            f64::from(width) / f64::from(COLS),
            f64::from(height) / f64::from(ROWS),
        );
        let gap = if w > 3.0 && h > 3.0 { 1.0 } else { 0.0 };
        self.marks = (0..ROWS)
            .flat_map(|row| (0..COLS).map(move |col| (row, col)))
            .map(|(row, col)| {
                let (x, y) = (f64::from(col) * w, f64::from(row) * h);
                Mark {
                    id: id(row, col),
                    rect: Rect::new(x, y, x + w - gap, y + h - gap),
                    fill: Color::from_rgba8(shade(col), shade(row), 150, 255),
                }
            })
            .collect();
    }

    fn at(&self, x: f32, y: f32) -> Option<u64> {
        self.marks
            .hit(Point::new(f64::from(x), f64::from(y)))
            .map(|m| m.id)
    }

    fn mark(&self, id: u64) -> Option<&Mark> {
        self.marks.as_slice().get(usize::try_from(id).ok()?)
    }
}

/// A cell's id: its row, then its column.
fn id(row: u32, col: u32) -> u64 {
    u64::from(row.saturating_mul(COLS).saturating_add(col))
}

/// A channel's value along a row or a column: 40 to 238.
fn shade(i: u32) -> u8 {
    u8::try_from(i.saturating_mul(2).saturating_add(40)).unwrap_or(u8::MAX)
}

/// Where an arrow key moves the pick from `at`: nowhere past an edge.
fn step(at: u64, key: &str) -> Option<u64> {
    let (cols, cells) = (u64::from(COLS), u64::from(COLS.saturating_mul(ROWS)));
    let col = at.checked_rem(cols)?;
    match key {
        "ArrowLeft" if col > 0 => at.checked_sub(1),
        "ArrowRight" if col.saturating_add(1) < cols => at.checked_add(1),
        "ArrowUp" => at.checked_sub(cols),
        "ArrowDown" => at.checked_add(cols).filter(|&to| to < cells),
        _ => None,
    }
}

/// `text` kept after what was typed, the last [`TYPED`] characters.
fn typed(state: &mut PaintState, text: &str) {
    state.typed.push_str(text);
    let over = state.typed.chars().count().saturating_sub(TYPED);
    if over > 0 {
        state.typed = state.typed.chars().skip(over).collect();
    }
}

impl PaintView for Grid {
    fn input(&mut self, _: &ViewCtx, event: &ViewInput) -> bool {
        let (hover, picked) = (self.hover, self.picked);
        let mut text = false;
        match event {
            ViewInput::PointerMove { x, y } => self.hover = self.at(*x, *y),
            ViewInput::PointerDown { x, y, .. } => self.picked = self.at(*x, *y),
            ViewInput::Touch { x, y, phase, .. } => {
                self.hover = self.at(*x, *y);
                if *phase == "start" {
                    self.picked = self.hover;
                }
            }
            ViewInput::Key {
                key,
                down: true,
                modifiers,
            } => {
                if let Some(to) = step(self.picked.unwrap_or(0), key) {
                    self.picked = Some(to);
                } else if key == "Backspace" {
                    self.state.update(|s| {
                        s.typed.pop();
                    });
                    text = true;
                } else if key.chars().count() == 1
                    && !modifiers
                        .iter()
                        .any(|m| matches!(*m, "Mod" | "Ctrl" | "Alt"))
                {
                    self.state.update(|s| typed(s, key));
                    text = true;
                }
            }
            ViewInput::Ime { preedit, commit } => {
                self.state.update(|s| {
                    s.preedit.clone_from(preedit);
                    if let Some(commit) = commit {
                        typed(s, commit);
                    }
                });
                text = true;
            }
            ViewInput::Focus(on) => self.state.update(|s| s.focused = *on),
            ViewInput::PointerUp { .. } | ViewInput::Wheel { .. } | ViewInput::Key { .. } => {}
        }
        let (now_hover, now_picked) = (self.hover, self.picked);
        self.state.update(|s| {
            s.events = s.events.saturating_add(1);
            s.hover = now_hover;
            s.picked = now_picked;
        });
        text || hover != now_hover || picked != now_picked
    }

    fn paint(&mut self, ctx: &ViewCtx, scene: &mut Scene) {
        let (width, height) = (ctx.geometry.width, ctx.geometry.height);
        self.lay_out(width, height);
        let all = Rect::new(0.0, 0.0, f64::from(width), f64::from(height));
        scene.fill(Fill::NonZero, Affine::IDENTITY, self.sheet, None, &all);
        self.marks.paint(scene, Affine::IDENTITY);
        // The lit and the picked drawn again over themselves: the same
        // rectangles the hit test finds.
        for (id, fill) in [(self.hover, self.lit), (self.picked, self.pick)] {
            if let Some(mark) = id.and_then(|id| self.mark(id)) {
                scene.fill(Fill::NonZero, Affine::IDENTITY, fill, None, &mark.rect);
            }
        }
    }

    fn hit(&self, _: &ViewCtx, at: Point) -> Option<u64> {
        self.marks.hit(at).map(|m| m.id)
    }
}

/// How often the line under the grid looks at the view's state.
const POLL: Duration = Duration::from_millis(100);

/// The pane: the grid, and a line of what it was given and how long its
/// frames take.
#[component]
pub fn PaintLabPane() -> Element {
    let t = use_shell().theme;
    let mount = use_hook(|| {
        let (state, stats) = shared();
        PaintMount::new(Grid::new)
            .extras(Extras::default().with_data(state.clone()))
            .stats(stats.clone())
    });
    let mut line = use_signal(status);
    use_future(move || async move {
        loop {
            futures_timer::Delay::new(POLL).await;
            let now = status();
            if *line.peek() != now {
                line.set(now);
            }
        }
    });
    rsx! {
        div { style: "flex: 1; display: flex; flex-direction: column; min-height: 0;",
            crate::embedded::KindHead {
                name: "Paint",
                kind: "paint",
                about: "10 000 rectangles drawn in-process by Blitz each frame; the pointer lights one, a press picks it, the arrows move it, and keys type.",
            }
            div { style: "flex: 1; position: relative; min-height: 0; margin: 4px 20px 8px; \
                          border: 1px solid {t.rule}; border-radius: 8px; overflow: hidden;",
                div { style: "position: absolute; inset: 0;",
                    PaintPane { mount }
                }
            }
            div { style: "padding: 0 20px 16px; font-size: 12px; color: {t.ink_3}; font-family: monospace;",
                "{line}"
            }
        }
    }
}

/// The line under the grid.
fn status() -> String {
    let (state, stats) = shared();
    let s = state.get();
    let f = stats.summary();
    let cell = |id: Option<u64>| id.map_or_else(|| "–".to_owned(), |id| id.to_string());
    format!(
        "under {} · picked {} · typed “{}{}” · {} frames, p50 {:.2} ms, p99 {:.2} ms",
        cell(s.hover),
        cell(s.picked),
        s.typed,
        s.preedit,
        f.frames,
        f.p50.as_secs_f64() * 1e3,
        f.p99.as_secs_f64() * 1e3,
    )
}

static PAINT_STATE: ActionMeta = ActionMeta {
    id: "LAB_PAINT_STATE",
    trait_name: "",
    method_name: "",
    display_name: "The paint pane's state",
    description: "The Embedded views' paint pane (the paint kind, 10 000 rectangles): the rectangle under the pointer and the picked one (ids, row * 100 + col), what was typed, whether it has the focus, the input events it was given, and its frames' count and p50 / p99 / slowest time in microseconds (the view's input and recording, on the host's clock).",
    category: "Lab",
    group: "Embedded views",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// A duration in whole microseconds.
fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// Puts `LAB_PAINT_STATE` on `list`.
pub fn register(list: &ActionList) {
    let _ = list.register_dynamic(
        &PAINT_STATE,
        Effect::ReadOnly,
        None,
        None,
        Arc::new(|_: &[u8]| {
            let (state, stats) = shared();
            let s = state.get();
            let f = stats.summary();
            let out = serde_json::json!({
                "hover": s.hover,
                "picked": s.picked,
                "typed": s.typed,
                "preedit": s.preedit,
                "focused": s.focused,
                "events": s.events,
                "frames": f.frames,
                "p50Us": micros(f.p50),
                "p99Us": micros(f.p99),
                "maxUs": micros(f.max),
            });
            Ok(serde_json::to_vec(&out).unwrap_or_default())
        }),
    );
}

#[cfg(test)]
mod tests {
    #![allow(clippy::arithmetic_side_effects)]

    use std::time::Instant;

    use architect::action::ActionCaller;
    use architect_workspace::{FrameClock, Geometry, KindId, Painter, PaneId, Placement, ViewHost};

    use super::*;

    fn painter(state: &HostData<PaintState>) -> Painter {
        let began = Instant::now();
        let ctx = ViewCtx::new(
            PaneId::new("embed-paint"),
            KindId::paint(),
            (ViewHost::Blitz, Placement::Pane),
            (ActionList::new(), ActionCaller::app()),
            FrameClock::new(Duration::from_millis(16), move || began.elapsed()),
        )
        .with_extras(Extras::default().with_data(state.clone()));
        let view = Grid::new(&ctx);
        Painter::new(ctx, view)
    }

    #[test]
    fn the_grid_takes_the_pointer_the_keys_and_composed_text() {
        let state = HostData::new(PaintState::default());
        let mut painter = painter(&state);
        let size = Geometry {
            width: 800.0,
            height: 600.0,
            scale: 2.0,
        };
        let _ = painter.frame(size);
        let input = painter.ctx().input.clone();

        // Column 12, row 34: what the pointer lights is what `hit` finds.
        input.push(ViewInput::PointerMove { x: 100.5, y: 206.5 });
        input.push(ViewInput::PointerDown {
            x: 100.5,
            y: 206.5,
            button: architect_workspace::Button::Primary,
        });
        let _ = painter.frame(size);
        let at = painter.hit(Point::new(100.5, 206.5));
        assert_eq!(at, Some(34 * 100 + 12));
        assert_eq!((state.get().hover, state.get().picked), (at, at));

        let key = |key: &str| ViewInput::Key {
            key: key.to_owned(),
            down: true,
            modifiers: Vec::new(),
        };
        input.push(key("ArrowRight"));
        input.push(key("ArrowDown"));
        input.push(ViewInput::Focus(true));
        input.push(key("h"));
        input.push(key("i"));
        input.push(ViewInput::Ime {
            preedit: "ka".to_owned(),
            commit: None,
        });
        input.push(ViewInput::Ime {
            preedit: String::new(),
            commit: Some("か".to_owned()),
        });
        assert!(painter.pump());
        let s = state.get();
        assert_eq!(s.picked, Some(35 * 100 + 13));
        assert_eq!(
            (s.typed.as_str(), s.preedit.as_str(), s.focused),
            ("hiか", "", true)
        );
        assert_eq!(s.events, 9);
        // At the right edge the arrow stops.
        assert_eq!(step(99, "ArrowRight"), None);
        assert_eq!(step(9_999, "ArrowDown"), None);
        assert_eq!(step(0, "ArrowUp"), None);
    }
}
