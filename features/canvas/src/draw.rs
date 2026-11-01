//! Shapes: what a layer or a window is drawn with — `gfx`'s lines,
//! rectangles, circles and text, in the points of what they are drawn on.

use crate::geometry::{Color, Point, Rect};

/// How wide a line of text at `size` is, in points.
///
/// An estimate the layout and a script's centring work with (half again
/// the size's width for every two characters), the same everywhere so a
/// test can pin it.
#[must_use]
pub fn text_width(text: &str, size: f32) -> f32 {
    // A label is short: its length fits an f32 exactly.
    #[allow(clippy::cast_precision_loss, clippy::as_conversions)]
    let chars = text.chars().count() as f32;
    chars * size * 0.55
}

/// One thing drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Line {
        from: Point,
        to: Point,
        color: Color,
        thickness: f32,
    },
    /// A rectangle: filled, or its edge `thickness` wide.
    Rect {
        rect: Rect,
        color: Color,
        rounding: f32,
        fill: bool,
        thickness: f32,
    },
    Circle {
        center: Point,
        radius: f32,
        color: Color,
        fill: bool,
        thickness: f32,
    },
    /// One line of text, its top left at `at`.
    Text {
        at: Point,
        text: String,
        color: Color,
        size: f32,
    },
}

/// Shapes in the order they are drawn: a later one over an earlier.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DrawList {
    shapes: Vec<Shape>,
}

impl DrawList {
    #[must_use]
    pub const fn new() -> Self {
        Self { shapes: Vec::new() }
    }

    #[must_use]
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    pub fn clear(&mut self) {
        self.shapes.clear();
    }

    pub fn push(&mut self, shape: Shape) {
        self.shapes.push(shape);
    }

    /// Puts `shape` under everything drawn so far.
    pub fn push_under(&mut self, shape: Shape) {
        self.shapes.insert(0, shape);
    }

    pub fn line(&mut self, from: Point, to: Point, color: Color, thickness: f32) {
        self.push(Shape::Line {
            from,
            to,
            color,
            thickness,
        });
    }

    pub fn rect_filled(&mut self, rect: Rect, color: Color, rounding: f32) {
        self.push(Shape::Rect {
            rect,
            color,
            rounding,
            fill: true,
            thickness: 0.0,
        });
    }

    pub fn rect(&mut self, rect: Rect, color: Color, rounding: f32, thickness: f32) {
        self.push(Shape::Rect {
            rect,
            color,
            rounding,
            fill: false,
            thickness,
        });
    }

    pub fn circle(&mut self, center: Point, radius: f32, color: Color, fill: bool) {
        self.push(Shape::Circle {
            center,
            radius,
            color,
            fill,
            thickness: 1.0,
        });
    }

    pub fn text(&mut self, at: Point, text: impl Into<String>, color: Color, size: f32) {
        self.push(Shape::Text {
            at,
            text: text.into(),
            color,
            size,
        });
    }
}
