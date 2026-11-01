//! The immediate-mode UI: windows and widgets a script draws every frame,
//! in `ReaImGui`'s shape — Dear `ImGui`'s API (MIT), re-declared here; nothing
//! is taken from `ReaImGui`'s (LGPL) files.
//!
//! ```text
//! ui.begin_frame(input);
//! let (visible, open) = ui.begin("Tool", "Tool##tool", true, 0);
//! if visible {
//!     let (changed, gain) = ui.slider_double("Gain", gain, 0.0, 1.0, "%.2f")?;
//!     ui.end();
//! }
//! let windows = ui.end_frame(); // each window's shapes, for its pane
//! ```
//!
//! A window is a workspace pane — docked, tabbed or floating, as the
//! person arranges it — so the UI does not move or title windows itself:
//! each is laid out from its own top left at its pane's size
//! ([`Input::sizes`]), the pane's tab is its title, and closing the pane
//! is what closes it ([`Input::closed`], `open` false from
//! [`Ui::begin`]). What the UI draws are [`Shape`](crate::Shape)s, drawn
//! by the host as a layer is.
//!
//! Interaction is Dear `ImGui`'s: an item is hovered when the pointer is
//! over it and nothing else is held; a press on it makes it active; a
//! button is clicked when the press is let go over it; a slider follows
//! the pointer while it is active. A host gives a frame for each press
//! and each release, so neither is lost between frames.

use std::collections::{BTreeMap, BTreeSet};

use crate::draw::{DrawList, text_width};
use crate::geometry::{Color, Point, Rect, Size};

/// Window flags (`ImGui.WindowFlags_*`), Dear `ImGui`'s values. Those a
/// pane makes moot (moving, resizing, collapsing, the title bar) are
/// taken and passed to the host, which may honour them.
pub mod window_flags {
    pub const NONE: u32 = 0;
    pub const NO_TITLE_BAR: u32 = 1 << 0;
    pub const NO_RESIZE: u32 = 1 << 1;
    pub const NO_MOVE: u32 = 1 << 2;
    pub const NO_SCROLLBAR: u32 = 1 << 3;
    pub const NO_COLLAPSE: u32 = 1 << 5;
    /// Its pane is sized to what it holds.
    pub const ALWAYS_AUTO_RESIZE: u32 = 1 << 6;
    /// No background drawn: what is under shows through.
    pub const NO_BACKGROUND: u32 = 1 << 7;
}

/// When a `SetNextWindow*` applies (`ImGui.Cond_*`), Dear `ImGui`'s values.
pub mod cond {
    pub const ALWAYS: u32 = 1 << 0;
    pub const ONCE: u32 = 1 << 1;
    pub const FIRST_USE_EVER: u32 = 1 << 2;
    pub const APPEARING: u32 = 1 << 3;
}

/// How the UI looks: colours and metrics, in points.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub text: Color,
    pub text_disabled: Color,
    pub window_bg: Color,
    pub frame_bg: Color,
    pub frame_hovered: Color,
    pub frame_active: Color,
    pub button: Color,
    pub button_hovered: Color,
    pub button_active: Color,
    pub slider_grab: Color,
    pub slider_grab_active: Color,
    pub check_mark: Color,
    pub separator: Color,
    /// A progress bar's fill.
    pub plot: Color,
    pub font_size: f32,
    /// Around a window's content.
    pub window_padding: Size,
    /// Inside a framed widget, around its text.
    pub frame_padding: Size,
    /// Between items: across on a line, down between lines.
    pub item_spacing: Size,
    /// Between a widget and its label.
    pub inner_spacing: f32,
    pub rounding: f32,
    pub grab_width: f32,
    /// A window's width before its pane says (first frame, no size asked).
    pub default_width: f32,
}

impl Default for Style {
    fn default() -> Self {
        let accent = Color(0x2DD4_BFFF);
        Self {
            text: Color(0xE4E4_E7FF),
            text_disabled: Color(0x8080_8AFF),
            window_bg: Color(0x0E0E_11FF),
            frame_bg: Color(0x1A1A_1FFF),
            frame_hovered: Color(0x2B2B_31FF),
            frame_active: Color(0x3434_3BFF),
            button: Color(0x2B2B_31FF),
            button_hovered: Color(0x3434_3BFF),
            button_active: accent.faded(0.5),
            slider_grab: accent,
            slider_grab_active: Color(0x5EEA_D4FF),
            check_mark: accent,
            separator: Color(0x2B2B_31FF),
            plot: accent,
            font_size: 13.0,
            window_padding: Size::new(8.0, 8.0),
            frame_padding: Size::new(6.0, 3.0),
            item_spacing: Size::new(8.0, 6.0),
            inner_spacing: 6.0,
            rounding: 4.0,
            grab_width: 12.0,
            default_width: 320.0,
        }
    }
}

impl Style {
    /// A framed widget's height: a line of text and its padding.
    #[must_use]
    pub const fn frame_height(&self) -> f32 {
        self.frame_padding.h.mul_add(2.0, self.font_size)
    }
}

/// The pointer over one window.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowPointer {
    /// The window's id ([`WindowFrame::id`]).
    pub window: String,
    /// In the window's points.
    pub at: Point,
}

/// What the host gives a frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Input {
    pub pointer: Option<WindowPointer>,
    /// The primary button is held.
    pub down: bool,
    /// Each shown window's size: its pane's.
    pub sizes: BTreeMap<String, Size>,
    /// Windows the person closed (their panes) since the last frame.
    pub closed: BTreeSet<String>,
}

/// One window, as drawn this frame.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowFrame {
    /// `<context>/<id>`: stable while the script names it the same, so a
    /// pane (and a screenset) can keep it.
    pub id: String,
    /// What its tab shows (the name before `##`).
    pub title: String,
    /// The size it is drawn at.
    pub size: Size,
    /// What it holds, padding included: an auto-resizing window's size.
    pub content: Size,
    /// Where it asked to be (`SetNextWindowPos`), for a floating pane.
    pub pos: Option<Point>,
    /// [`window_flags`].
    pub flags: u32,
    /// It has a close button: closing its pane ends it.
    pub closable: bool,
    pub draw: DrawList,
}

/// Why a call was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiError {
    /// A widget with no window begun (its name).
    NoWindow(&'static str),
    /// `PopID` with nothing pushed.
    IdStack,
}

impl std::fmt::Display for UiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWindow(what) => write!(f, "{what} outside a window: call Begin first"),
            Self::IdStack => f.write_str("PopID without a PushID"),
        }
    }
}

impl std::error::Error for UiError {}

/// The last item drawn, for `IsItemHovered` and friends.
#[derive(Clone, Copy, Debug, Default)]
struct Item {
    hovered: bool,
    active: bool,
}

#[derive(Clone, Copy, Debug)]
enum SameLine {
    /// After the last item, this far.
    After(f32),
    /// At this x from the window's left.
    At(f32),
}

#[derive(Clone, Copy, Debug)]
struct Line {
    right: f32,
    top: f32,
    height: f32,
}

/// A window being drawn.
#[derive(Debug)]
struct Building {
    id: String,
    title: String,
    flags: u32,
    closable: bool,
    size: Option<Size>,
    pos: Option<Point>,
    draw: DrawList,
    next_y: f32,
    line: Option<Line>,
    same_line: Option<SameLine>,
    content: Size,
    last: Item,
    ids: Vec<u64>,
}

/// What `SetNextWindow*` asked for.
#[derive(Clone, Copy, Debug, Default)]
struct Next {
    size: Option<(Size, u32)>,
    pos: Option<(Point, u32)>,
}

/// One script's immediate-mode UI: its windows, across every context.
#[derive(Debug, Default)]
pub struct Ui {
    style: Style,
    input: Input,
    was_down: bool,
    /// The item held (pressed and not let go).
    active: Option<u64>,
    stack: Vec<Building>,
    done: Vec<WindowFrame>,
    next: Next,
    /// Windows begun before, for the `Cond_*` that apply once.
    seen: BTreeSet<String>,
    /// Sizes and places asked for once, kept.
    kept_size: BTreeMap<String, Size>,
    kept_pos: BTreeMap<String, Point>,
}

impl Ui {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn style(&self) -> &Style {
        &self.style
    }

    pub const fn set_style(&mut self, style: Style) {
        self.style = style;
    }

    /// Starts a frame with what the host gives it.
    pub fn begin_frame(&mut self, input: Input) {
        self.input = input;
        self.stack.clear();
        self.done.clear();
        self.next = Next::default();
    }

    /// Ends the frame: every window begun, in order (one not ended is
    /// ended here).
    pub fn end_frame(&mut self) -> Vec<WindowFrame> {
        while !self.stack.is_empty() {
            self.end();
        }
        if !self.input.down {
            self.active = None;
        }
        self.was_down = self.input.down;
        std::mem::take(&mut self.done)
    }

    /// `SetNextWindowSize`: the next window's size, when `cond` says
    /// ([`cond`]; 0 is always). A pane's own size wins over it.
    pub const fn set_next_window_size(&mut self, size: Size, cond: u32) {
        self.next.size = Some((size, cond));
    }

    /// `SetNextWindowPos`: where the next window floats when it opens.
    pub const fn set_next_window_pos(&mut self, pos: Point, cond: u32) {
        self.next.pos = Some((pos, cond));
    }

    /// `Begin`: starts window `name` (`Title##id`: the tab shows `Title`,
    /// the id is the whole; `###id`, the id only) of `context`. It has a
    /// close button when `closable`. Gives (`visible`, `open`): draw into
    /// it and call [`Self::end`] when `visible`; `open` turns false the
    /// frame its pane is closed.
    pub fn begin(&mut self, context: &str, name: &str, closable: bool, flags: u32) -> (bool, bool) {
        let (title, key) = split_label(name);
        let id = format!("{context}/{key}");
        let next = std::mem::take(&mut self.next);
        if closable && self.input.closed.contains(&id) {
            return (false, false);
        }
        let first = self.seen.insert(id.clone());
        if let Some((size, when)) = next.size
            && applies(when, first)
        {
            self.kept_size.insert(id.clone(), size);
        }
        if let Some((pos, when)) = next.pos
            && applies(when, first)
        {
            self.kept_pos.insert(id.clone(), pos);
        }
        let size = self
            .input
            .sizes
            .get(&id)
            .or_else(|| self.kept_size.get(&id))
            .copied();
        let seed = hash(0, id.as_bytes());
        self.stack.push(Building {
            title: title.to_owned(),
            pos: self.kept_pos.get(&id).copied(),
            id,
            flags,
            closable,
            size,
            draw: DrawList::new(),
            next_y: self.style.window_padding.h,
            line: None,
            same_line: None,
            content: Size::default(),
            last: Item::default(),
            ids: vec![seed],
        });
        (true, true)
    }

    /// `End`: the window begun last is done. Nothing when none is.
    pub fn end(&mut self) {
        let Some(mut w) = self.stack.pop() else {
            return;
        };
        let pad = self.style.window_padding;
        let content = Size::new(w.content.w + pad.w, w.content.h + pad.h);
        let auto = w.flags & window_flags::ALWAYS_AUTO_RESIZE != 0;
        let size = match (w.size, auto) {
            (Some(size), false) => size,
            (_, true) => content,
            (None, false) => Size::new(self.style.default_width.max(content.w), content.h),
        };
        if w.flags & window_flags::NO_BACKGROUND == 0 {
            w.draw.push_under(crate::Shape::Rect {
                rect: Rect::new(0.0, 0.0, size.w, size.h),
                color: self.style.window_bg,
                rounding: 0.0,
                fill: true,
                thickness: 0.0,
            });
        }
        self.done.push(WindowFrame {
            id: w.id,
            title: w.title,
            size,
            content,
            pos: w.pos,
            flags: w.flags,
            closable: w.closable,
            draw: w.draw,
        });
    }

    /// `PushID`: what follows is told apart by `id` too (the same label in
    /// each row of a loop).
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn push_id(&mut self, id: &str) -> Result<(), UiError> {
        let w = self.current("PushID")?;
        let top = w.ids.last().copied().unwrap_or_default();
        w.ids.push(hash(top, id.as_bytes()));
        Ok(())
    }

    /// `PopID`.
    ///
    /// # Errors
    ///
    /// No window is begun, or nothing was pushed.
    pub fn pop_id(&mut self) -> Result<(), UiError> {
        let w = self.current("PopID")?;
        if w.ids.len() <= 1 {
            return Err(UiError::IdStack);
        }
        w.ids.pop();
        Ok(())
    }

    /// `SameLine`: the next item goes on this line — `offset` from the
    /// window's left when above zero, else `spacing` after this item (the
    /// style's when below zero).
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn same_line(&mut self, offset: f32, spacing: f32) -> Result<(), UiError> {
        let default = self.style.item_spacing.w;
        let pad = self.style.window_padding.w;
        let w = self.current("SameLine")?;
        w.same_line = Some(if offset > 0.0 {
            SameLine::At(pad + offset)
        } else {
            SameLine::After(if spacing < 0.0 { default } else { spacing })
        });
        Ok(())
    }

    /// `Spacing`: a gap down.
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn spacing(&mut self) -> Result<(), UiError> {
        let gap = self.style.item_spacing.h;
        let w = self.current("Spacing")?;
        w.next_y += gap;
        w.line = None;
        w.same_line = None;
        Ok(())
    }

    /// `Separator`: a rule across the window.
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn separator(&mut self) -> Result<(), UiError> {
        let color = self.style.separator;
        let width = self.width("Separator")?;
        let (style, w) = self.parts("Separator")?;
        w.same_line = None;
        let rect = place(w, style, 0.0, 1.0);
        w.draw.line(
            Point::new(0.0, rect.y),
            Point::new(width, rect.y),
            color,
            1.0,
        );
        w.last = Item::default();
        Ok(())
    }

    /// `Text`.
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn text(&mut self, text: &str) -> Result<(), UiError> {
        let color = self.style.text;
        self.text_colored(color, text)
    }

    /// `TextDisabled`: in the faint colour.
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn text_disabled(&mut self, text: &str) -> Result<(), UiError> {
        let color = self.style.text_disabled;
        self.text_colored(color, text)
    }

    /// `TextColored`.
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn text_colored(&mut self, color: Color, text: &str) -> Result<(), UiError> {
        let size = self.style.font_size;
        let (style, w) = self.parts("Text")?;
        let rect = place(w, style, text_width(text, size), size);
        w.draw.text(rect.origin(), text, color, size);
        w.last = Item::default();
        Ok(())
    }

    /// `Button`: whether it was clicked. A size of 0 fits its label.
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn button(&mut self, label: &str, size: Size) -> Result<bool, UiError> {
        let s = self.style.clone();
        let (shown, key) = split_label(label);
        let w = self.current("Button")?;
        let id = item_id(w, key);
        let width = if size.w > 0.0 {
            size.w
        } else {
            s.frame_padding
                .w
                .mul_add(2.0, text_width(shown, s.font_size))
        };
        let height = if size.h > 0.0 {
            size.h
        } else {
            s.frame_height()
        };
        let rect = place(w, &s, width, height);
        let state = self.interact(id, rect);
        let fill = match (state.held, state.hovered) {
            (true, _) => s.button_active,
            (false, true) => s.button_hovered,
            _ => s.button,
        };
        let w = self.current("Button")?;
        w.draw.rect_filled(rect, fill, s.rounding);
        let tw = text_width(shown, s.font_size);
        w.draw.text(
            Point::new(
                rect.x + (rect.w - tw) / 2.0,
                rect.y + (rect.h - s.font_size) / 2.0,
            ),
            shown,
            s.text,
            s.font_size,
        );
        w.last = state.item();
        Ok(state.clicked)
    }

    /// `Checkbox`: (`changed`, the value now).
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn checkbox(&mut self, label: &str, value: bool) -> Result<(bool, bool), UiError> {
        let s = self.style.clone();
        let (shown, key) = split_label(label);
        let w = self.current("Checkbox")?;
        let id = item_id(w, key);
        let side = s.frame_height();
        let label_w = if shown.is_empty() {
            0.0
        } else {
            s.inner_spacing + text_width(shown, s.font_size)
        };
        let rect = place(w, &s, side + label_w, side);
        let state = self.interact(id, rect);
        let value = if state.clicked { !value } else { value };
        let w = self.current("Checkbox")?;
        let square = Rect::new(rect.x, rect.y, side, side);
        w.draw
            .rect_filled(square, frame_fill(&s, state), s.rounding);
        if value {
            let inset = (side / 4.0).max(2.0);
            w.draw.rect_filled(
                Rect::new(
                    square.x + inset,
                    square.y + inset,
                    inset.mul_add(-2.0, side),
                    inset.mul_add(-2.0, side),
                ),
                s.check_mark,
                s.rounding / 2.0,
            );
        }
        label_after(&mut w.draw, &s, square, shown);
        w.last = state.item();
        Ok((state.clicked, value))
    }

    /// `SliderDouble`: (`changed`, the value now), dragged across
    /// `min..=max` and shown with `format` (`%.3f` when empty).
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn slider_double(
        &mut self,
        label: &str,
        value: f64,
        min: f64,
        max: f64,
        format: &str,
    ) -> Result<(bool, f64), UiError> {
        let format = if format.is_empty() { "%.3f" } else { format };
        self.slider("SliderDouble", label, value, min, max, format, false)
    }

    /// `SliderInt`: as [`Self::slider_double`], in whole steps (`%d`).
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn slider_int(
        &mut self,
        label: &str,
        value: i64,
        min: i64,
        max: i64,
        format: &str,
    ) -> Result<(bool, i64), UiError> {
        let format = if format.is_empty() { "%d" } else { format };
        // Slider bounds a person drags between: well inside f64's exact
        // integers, and back.
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::as_conversions
        )]
        {
            let (changed, v) = self.slider(
                "SliderInt",
                label,
                value as f64,
                min as f64,
                max as f64,
                format,
                true,
            )?;
            Ok((changed, v.round() as i64))
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn slider(
        &mut self,
        what: &'static str,
        label: &str,
        value: f64,
        min: f64,
        max: f64,
        format: &str,
        whole: bool,
    ) -> Result<(bool, f64), UiError> {
        let s = self.style.clone();
        let (shown, key) = split_label(label);
        let width = self.width(what)?;
        let w = self.current(what)?;
        let id = item_id(w, key);
        let frame_w = (s.window_padding.w.mul_add(-2.0, width) * 0.65).max(s.grab_width * 4.0);
        let label_w = if shown.is_empty() {
            0.0
        } else {
            s.inner_spacing + text_width(shown, s.font_size)
        };
        let full = place(w, &s, frame_w + label_w, s.frame_height());
        let frame = Rect::new(full.x, full.y, frame_w, full.h);
        let state = self.interact(id, frame);
        let span = max - min;
        let travel = (frame.w - s.grab_width).max(1.0);
        let mut next = value;
        if state.held
            && let Some(p) = self.pointer_in_current()
        {
            let t = f64::from(((p.x - frame.x - s.grab_width / 2.0) / travel).clamp(0.0, 1.0));
            next = t.mul_add(span, min);
            if whole {
                next = next.round();
            }
        }
        let t = if span == 0.0 {
            0.0
        } else {
            ((next - min) / span).clamp(0.0, 1.0)
        };
        let w = self.current(what)?;
        w.draw.rect_filled(frame, frame_fill(&s, state), s.rounding);
        // A fraction of a slider's travel: points.
        #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
        let grab_x = (t as f32).mul_add(travel, frame.x);
        w.draw.rect_filled(
            Rect::new(grab_x, frame.y + 2.0, s.grab_width, frame.h - 4.0),
            if state.held {
                s.slider_grab_active
            } else {
                s.slider_grab
            },
            s.rounding / 2.0,
        );
        let text = format_value(format, next, whole);
        let tw = text_width(&text, s.font_size);
        w.draw.text(
            Point::new(frame.x + (frame.w - tw) / 2.0, frame.y + s.frame_padding.h),
            text,
            s.text,
            s.font_size,
        );
        label_after(&mut w.draw, &s, frame, shown);
        w.last = state.item();
        // Exactly: a slider not moved gives back the value it was given.
        #[allow(clippy::float_cmp)]
        let changed = next != value;
        Ok((changed, next))
    }

    /// `ProgressBar`: `fraction` (0..=1) of a bar, `overlay` over it (the
    /// percentage when empty). A width of 0 fills the line.
    ///
    /// # Errors
    ///
    /// No window is begun.
    pub fn progress_bar(
        &mut self,
        fraction: f32,
        size: Size,
        overlay: &str,
    ) -> Result<(), UiError> {
        let s = self.style.clone();
        let width = self.width("ProgressBar")?;
        let w = self.current("ProgressBar")?;
        let bar_w = if size.w > 0.0 {
            size.w
        } else {
            s.window_padding.w.mul_add(-2.0, width).max(s.grab_width)
        };
        let bar_h = if size.h > 0.0 {
            size.h
        } else {
            s.frame_height()
        };
        let rect = place(w, &s, bar_w, bar_h);
        let fraction = fraction.clamp(0.0, 1.0);
        w.draw.rect_filled(rect, s.frame_bg, s.rounding);
        w.draw.rect_filled(
            Rect::new(rect.x, rect.y, rect.w * fraction, rect.h),
            s.plot,
            s.rounding,
        );
        let text = if overlay.is_empty() {
            format!("{:.0}%", fraction * 100.0)
        } else {
            overlay.to_owned()
        };
        let tw = text_width(&text, s.font_size);
        w.draw.text(
            Point::new(
                rect.x + (rect.w - tw) / 2.0,
                rect.y + (rect.h - s.font_size) / 2.0,
            ),
            text,
            s.text,
            s.font_size,
        );
        w.last = Item::default();
        Ok(())
    }

    /// `IsItemHovered`: the last item.
    #[must_use]
    pub fn is_item_hovered(&self) -> bool {
        self.stack.last().is_some_and(|w| w.last.hovered)
    }

    /// `IsItemActive`: the last item is held.
    #[must_use]
    pub fn is_item_active(&self) -> bool {
        self.stack.last().is_some_and(|w| w.last.active)
    }

    fn current(&mut self, what: &'static str) -> Result<&mut Building, UiError> {
        self.stack.last_mut().ok_or(UiError::NoWindow(what))
    }

    /// The style, and the window being drawn.
    fn parts(&mut self, what: &'static str) -> Result<(&Style, &mut Building), UiError> {
        let w = self.stack.last_mut().ok_or(UiError::NoWindow(what))?;
        Ok((&self.style, w))
    }

    /// The current window's width: its pane's, or the default.
    fn width(&mut self, what: &'static str) -> Result<f32, UiError> {
        let default = self.style.default_width;
        Ok(self.current(what)?.size.map_or(default, |s| s.w))
    }

    fn pointer_in_current(&self) -> Option<Point> {
        let w = self.stack.last()?;
        let p = self.input.pointer.as_ref()?;
        (p.window == w.id).then_some(p.at)
    }

    /// What the pointer does to item `id` at `rect` this frame.
    fn interact(&mut self, id: u64, rect: Rect) -> Interaction {
        let over = self.pointer_in_current().is_some_and(|p| rect.contains(p));
        let free = self.active.is_none_or(|a| a == id);
        let hovered = over && free;
        let (down, was) = (self.input.down, self.was_down);
        if hovered && down && !was {
            self.active = Some(id);
        }
        let ours = self.active == Some(id);
        Interaction {
            hovered,
            held: ours && down,
            clicked: ours && !down && was && over,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Interaction {
    hovered: bool,
    held: bool,
    clicked: bool,
}

impl Interaction {
    const fn item(self) -> Item {
        Item {
            hovered: self.hovered,
            active: self.held,
        }
    }
}

const fn frame_fill(s: &Style, state: Interaction) -> Color {
    match (state.held, state.hovered) {
        (true, _) => s.frame_active,
        (false, true) => s.frame_hovered,
        _ => s.frame_bg,
    }
}

/// A widget's label, to the right of its frame.
fn label_after(draw: &mut DrawList, s: &Style, frame: Rect, label: &str) {
    if !label.is_empty() {
        draw.text(
            Point::new(frame.right() + s.inner_spacing, frame.y + s.frame_padding.h),
            label,
            s.text,
            s.font_size,
        );
    }
}

/// Places an item `w` by `h` in window `b`: on the line when `SameLine`
/// asked, else below the last line.
fn place(window: &mut Building, style: &Style, width: f32, height: f32) -> Rect {
    let (left, top, line_height) = match (window.same_line.take(), window.line) {
        (Some(SameLine::After(gap)), Some(line)) => {
            (line.right + gap, line.top, line.height.max(height))
        }
        (Some(SameLine::At(left)), Some(line)) => (left, line.top, line.height.max(height)),
        _ => (style.window_padding.w, window.next_y, height),
    };
    let rect = Rect::new(left, top, width, height);
    window.line = Some(Line {
        right: rect.right(),
        top,
        height: line_height,
    });
    window.next_y = top + line_height + style.item_spacing.h;
    window.content.w = window.content.w.max(rect.right());
    window.content.h = window.content.h.max(top + line_height);
    rect
}

fn item_id(b: &Building, key: &str) -> u64 {
    hash(b.ids.last().copied().unwrap_or_default(), key.as_bytes())
}

/// Whether a `SetNextWindow*` with `cond` applies, the window `first`
/// begun now or not.
const fn applies(cond: u32, first: bool) -> bool {
    cond == 0 || cond & cond::ALWAYS != 0 || first
}

/// `Title##id` as what is shown and what is hashed: `Title` and the
/// whole; `Title###id`, `Title` and `id`.
#[must_use]
pub fn split_label(label: &str) -> (&str, &str) {
    if let Some((shown, id)) = label.split_once("###") {
        return (shown, id);
    }
    match label.split_once("##") {
        Some((shown, _)) => (shown, label),
        None => (label, label),
    }
}

/// FNV-1a, seeded: an id that is the same on every run.
fn hash(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed ^ 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

/// `value` through a printf format's first conversion (`%d`, `%.2f`,
/// `%5.1f dB`, `%g`, `%%`); the text around it kept.
#[must_use]
pub fn format_value(format: &str, value: f64, whole: bool) -> String {
    let mut out = String::with_capacity(format.len().saturating_add(8));
    let mut chars = format.chars().peekable();
    let mut done = false;
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }
        let mut spec = String::new();
        let mut conversion = None;
        for next in chars.by_ref() {
            if next.is_ascii_alphabetic() {
                conversion = Some(next);
                break;
            }
            spec.push(next);
        }
        if done {
            continue;
        }
        done = true;
        let precision = spec
            .split_once('.')
            .and_then(|(_, p)| p.parse::<usize>().ok());
        let width = spec
            .split('.')
            .next()
            .and_then(|w| {
                w.trim_start_matches(['-', '+', ' ', '0'])
                    .parse::<usize>()
                    .ok()
            })
            .unwrap_or(0);
        let text = match conversion {
            Some('d' | 'i' | 'u') => format!("{value:.0}"),
            Some('g') => format!("{value}"),
            _ if whole => format!("{value:.0}"),
            _ => format!("{value:.*}", precision.unwrap_or(6)),
        };
        out.extend(std::iter::repeat_n(
            ' ',
            width.saturating_sub(text.chars().count()),
        ));
        out.push_str(&text);
    }
    out
}

#[cfg(test)]
#[allow(clippy::unnecessary_wraps, clippy::suboptimal_flops)]
mod tests {
    use super::{Input, Ui, WindowPointer, format_value, split_label, window_flags};
    use crate::Shape;
    use crate::geometry::{Point, Size};

    const CTX: &str = "tool";

    fn at(window: &str, x: f32, y: f32) -> Option<WindowPointer> {
        Some(WindowPointer {
            window: format!("{CTX}/{window}"),
            at: Point::new(x, y),
        })
    }

    #[test]
    fn a_label_shows_before_its_hashes() {
        assert_eq!(split_label("Gain##g1"), ("Gain", "Gain##g1"));
        assert_eq!(split_label("Gain###g"), ("Gain", "g"));
        assert_eq!(split_label("Gain"), ("Gain", "Gain"));
    }

    #[test]
    fn printf_formats_render_like_imgui() {
        assert_eq!(format_value("%.3f", 0.5, false), "0.500");
        assert_eq!(format_value("%d", 3.0, true), "3");
        assert_eq!(format_value("%.1f dB", -6.04, false), "-6.0 dB");
        assert_eq!(format_value("%5.1f", 2.0, false), "  2.0");
        assert_eq!(format_value("%d%%", 40.0, true), "40%");
        assert_eq!(format_value("bars", 4.0, true), "bars");
    }

    /// One frame of a window with a slider and a button.
    fn tool(ui: &mut Ui, input: Input, gain: f64) -> (f64, bool, Vec<super::WindowFrame>) {
        ui.begin_frame(input);
        let (visible, _) = ui.begin(CTX, "Tool##t", true, 0);
        assert!(visible);
        let (_, gain) = ui
            .slider_double("Gain", gain, 0.0, 1.0, "%.2f")
            .unwrap_or_default();
        let pressed = ui.button("Reset", Size::default()).unwrap_or_default();
        ui.end();
        (gain, pressed, ui.end_frame())
    }

    #[test]
    fn a_slider_follows_a_drag_and_a_button_clicks_on_release() {
        let mut ui = Ui::new();
        let sized = |down, pointer| Input {
            pointer,
            down,
            sizes: [(format!("{CTX}/Tool##t"), Size::new(300.0, 120.0))].into(),
            ..Input::default()
        };
        let (gain, _, frames) = tool(&mut ui, sized(false, None), 0.25);
        assert_eq!(gain, 0.25);
        let window = frames.first().map(|w| (w.title.as_str(), w.size));
        assert_eq!(window, Some(("Tool", Size::new(300.0, 120.0))));
        // The slider's frame is 0.65 of the content width, from (8, 8).
        let frame_w = (300.0 - 16.0) * 0.65;
        let (gain, _, _) = tool(&mut ui, sized(true, at("Tool##t", 10.0, 12.0)), gain);
        assert_eq!(gain, 0.0, "pressed at its left end");
        let (gain, _, _) = tool(
            &mut ui,
            sized(true, at("Tool##t", 8.0 + frame_w, 12.0)),
            gain,
        );
        assert_eq!(gain, 1.0, "dragged to its right end");
        // Dragged past the window, it stays held and at its end.
        let (gain, _, _) = tool(&mut ui, sized(true, at("Tool##t", 999.0, 99.0)), gain);
        assert_eq!(gain, 1.0);
        let (gain, _, _) = tool(&mut ui, sized(false, at("Tool##t", 999.0, 99.0)), gain);
        // The button is the next line: y = 8 + 19 + 6.
        let on_button = at("Tool##t", 12.0, 36.0);
        let (_, pressed, _) = tool(&mut ui, sized(true, on_button.clone()), gain);
        assert!(!pressed, "a click is on release");
        let (_, pressed, _) = tool(&mut ui, sized(false, on_button), gain);
        assert!(pressed);
    }

    #[test]
    fn closing_its_pane_closes_a_window_and_auto_resize_fits_it() {
        let mut ui = Ui::new();
        ui.begin_frame(Input {
            closed: [format!("{CTX}/Tool")].into(),
            ..Input::default()
        });
        assert_eq!(ui.begin(CTX, "Tool", true, 0), (false, false));
        assert!(ui.end_frame().is_empty());
        ui.begin_frame(Input::default());
        ui.begin(CTX, "Fit", false, window_flags::ALWAYS_AUTO_RESIZE);
        ui.text("hello").ok();
        ui.end();
        let frames = ui.end_frame();
        let fit = frames.first().map(|w| w.size).unwrap_or_default();
        // 5 chars at 13pt × 0.55, padded 8 each side; one line padded.
        assert!((fit.w - (5.0 * 13.0 * 0.55 + 16.0)).abs() < 1e-3, "{fit:?}");
        assert!((fit.h - (13.0 + 16.0)).abs() < 1e-3, "{fit:?}");
        let first = frames
            .first()
            .and_then(|w| w.draw.shapes().first().cloned());
        assert!(
            matches!(first, Some(Shape::Rect { fill: true, .. })),
            "its background is under"
        );
        assert!(ui.text("no window").is_err());
    }

    #[test]
    fn a_checkbox_toggles_and_same_line_puts_items_side_by_side() {
        let mut ui = Ui::new();
        let mut on = false;
        let frame = |ui: &mut Ui, down, on: bool| {
            ui.begin_frame(Input {
                pointer: at("W", 10.0, 10.0),
                down,
                ..Input::default()
            });
            ui.begin(CTX, "W", false, 0);
            let (_, on) = ui.checkbox("Loop", on).unwrap_or_default();
            ui.same_line(0.0, -1.0).ok();
            ui.text("after").ok();
            ui.end();
            (on, ui.end_frame())
        };
        (on, _) = frame(&mut ui, true, on);
        (on, _) = frame(&mut ui, false, on);
        assert!(on);
        let (_, frames) = frame(&mut ui, false, on);
        let text_at = frames.first().and_then(|w| {
            w.draw.shapes().iter().find_map(|s| match s {
                Shape::Text { text, at, .. } if text == "after" => Some(*at),
                _ => None,
            })
        });
        // The box (19) and its label ("Loop", 6 + 28.6) then 8 across.
        let x = 8.0 + 19.0 + 6.0 + 4.0 * 13.0 * 0.55 + 8.0;
        assert!(
            text_at.is_some_and(|p| (p.x - x).abs() < 1e-3 && (p.y - 8.0).abs() < 1e-3),
            "{text_at:?}"
        );
    }
}
