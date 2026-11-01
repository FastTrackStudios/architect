//! Layers: what an extension or a script draws over a view, in the view's
//! coordinates, and the regions of it that take the pointer.
//!
//! A layer is contributed to a view by its kind (`session.arrange`), on a
//! [`Plane`] of it — under the view's items, over them, or over
//! everything — and the view mounts each plane's layers where that plane
//! is, clipped to itself. A layer passes the pointer through to the view
//! everywhere except the [`Region`]s it claims: a marker's flag it can be
//! dragged by, a label to click. Which region is under a point is
//! [`Layer::hit`]: the last claimed, as the last drawn is on top.
//!
//! [`Overlays`] is the state behind them, immediate-mode: each frame a
//! layer is drawn into it is drawn afresh, and a region asked about
//! answers from the pointer the view last reported.

use std::collections::BTreeMap;

use crate::draw::DrawList;
use crate::geometry::{Point, Rect};
use crate::space::Spaces;

/// Where in a view a layer is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Plane {
    /// Under the view's items (a lane's wash, grid lines).
    Under,
    /// Over its items, under its own overlays (markers, labels).
    #[default]
    Over,
    /// Over everything the view draws (a playhead's glow).
    Top,
}

impl Plane {
    /// From its name: `under`, `over`, `top`.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "under" => Some(Self::Under),
            "over" => Some(Self::Over),
            "top" => Some(Self::Top),
            _ => None,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Under => "under",
            Self::Over => "over",
            Self::Top => "top",
        }
    }
}

/// A part of a layer that takes the pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// Its layer's own name for it (`marker:Chorus`).
    pub id: String,
    /// In the view's points.
    pub rect: Rect,
}

/// One layer, as drawn.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    /// Unique among its owner's layers.
    pub id: String,
    /// The kind of view it is drawn over.
    pub view: String,
    pub plane: Plane,
    pub draw: DrawList,
    /// What takes the pointer, in the order claimed.
    pub regions: Vec<Region>,
}

impl Layer {
    #[must_use]
    pub fn new(id: impl Into<String>, view: impl Into<String>, plane: Plane) -> Self {
        Self {
            id: id.into(),
            view: view.into(),
            plane,
            draw: DrawList::new(),
            regions: Vec::new(),
        }
    }

    /// The region under `p`: the last claimed, which is drawn on top.
    /// `None` passes the pointer through to the view.
    #[must_use]
    pub fn hit(&self, p: Point) -> Option<&Region> {
        hit(&self.regions, p)
    }
}

/// The pointer over a view, as its view reports it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ViewPointer {
    /// Where it is, in the view's points; `None` when it is not over it.
    pub at: Option<Point>,
    /// The primary button is held.
    pub down: bool,
    /// A secondary click (a context menu) since the last frame.
    pub context: bool,
}

/// What a region knows of the pointer this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegionState {
    /// The pointer is over it, and over no region claimed above it.
    pub hovered: bool,
    /// Pressed and let go over it.
    pub clicked: bool,
    /// Held: pressed on it and not let go (a drag).
    pub held: bool,
    /// Secondary-clicked over it.
    pub context: bool,
}

/// A layer an [`Overlays`] keeps, by its index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct LayerId(pub usize);

#[derive(Debug, Default)]
struct ViewState {
    spaces: Option<Spaces>,
    pointer: ViewPointer,
    was_down: bool,
    /// The region the primary press began on: its layer and id.
    pressed: Option<(LayerId, String)>,
}

#[derive(Debug)]
struct Kept {
    layer: Layer,
    /// Drawn into this frame: the next draw does not clear it again.
    fresh: bool,
    /// What it claimed last frame: the hit-test this frame's questions
    /// answer from (what is on top is only known once all is drawn).
    last: Vec<Region>,
}

/// One owner's layers over every view, and what each view reported.
#[derive(Debug, Default)]
pub struct Overlays {
    views: BTreeMap<String, ViewState>,
    layers: Vec<Kept>,
}

impl Overlays {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A view of kind `view` shows with these coordinates (or no longer
    /// does, with `None`).
    pub fn publish(&mut self, view: &str, spaces: Option<Spaces>) {
        self.views.entry(view.to_owned()).or_default().spaces = spaces;
    }

    /// What `view` published, while it shows.
    #[must_use]
    pub fn spaces(&self, view: &str) -> Option<&Spaces> {
        self.views.get(view)?.spaces.as_ref()
    }

    /// The pointer over `view` now.
    pub fn pointer(&mut self, view: &str, pointer: ViewPointer) {
        let state = self.views.entry(view.to_owned()).or_default();
        // A context click stays until a frame has seen it.
        let context = state.pointer.context || pointer.context;
        state.pointer = ViewPointer { context, ..pointer };
    }

    /// The layer `id` over `view` on `plane`: made the first time, the
    /// same one after (moved to `plane` if it changed).
    pub fn layer(&mut self, id: &str, view: &str, plane: Plane) -> LayerId {
        if let Some(found) = self
            .layers
            .iter()
            .position(|k| k.layer.id == id && k.layer.view == view)
        {
            if let Some(kept) = self.layers.get_mut(found) {
                kept.layer.plane = plane;
            }
            return LayerId(found);
        }
        self.layers.push(Kept {
            layer: Layer::new(id, view, plane),
            fresh: false,
            last: Vec::new(),
        });
        LayerId(self.layers.len().saturating_sub(1))
    }

    /// What `layer` is drawn over: its view's kind and coordinates.
    #[must_use]
    pub fn view_of(&self, layer: LayerId) -> Option<(&str, Option<&Spaces>)> {
        let kept = self.layers.get(layer.0)?;
        let view = kept.layer.view.as_str();
        Some((view, self.spaces(view)))
    }

    /// Its shapes to draw into: emptied by the first draw of a frame, so
    /// a frame that draws a layer draws all of it, and one that does not
    /// leaves it as it was.
    pub fn draw(&mut self, layer: LayerId) -> Option<&mut DrawList> {
        let kept = self.layers.get_mut(layer.0)?;
        fresh(kept);
        Some(&mut kept.layer.draw)
    }

    /// Empties `layer` now.
    pub fn clear(&mut self, layer: LayerId) {
        if let Some(kept) = self.layers.get_mut(layer.0) {
            fresh(kept);
            kept.layer.draw.clear();
            kept.layer.regions.clear();
        }
    }

    /// Claims `rect` of `layer` as region `id`, and says what the pointer
    /// is doing to it.
    pub fn region(&mut self, layer: LayerId, id: &str, rect: Rect) -> RegionState {
        let Some(kept) = self.layers.get_mut(layer.0) else {
            return RegionState::default();
        };
        fresh(kept);
        kept.layer.regions.push(Region {
            id: id.to_owned(),
            rect,
        });
        // On top last frame, or — new this frame — over nothing claimed
        // then.
        let top = |p: Point| hit(&kept.last, p).is_none_or(|r| r.id == id);
        let Some(view) = self.views.get(&kept.layer.view) else {
            return RegionState::default();
        };
        let p = view.pointer;
        let hovered = p.at.is_some_and(|at| rect.contains(at) && top(at));
        let ours = view
            .pressed
            .as_ref()
            .is_some_and(|(l, r)| *l == layer && r == id);
        RegionState {
            hovered,
            clicked: ours && !p.down && hovered,
            held: ours && p.down,
            context: hovered && p.context,
        }
    }

    /// Starts a frame: a press that began this frame is given to the
    /// region on top under it.
    pub fn begin_frame(&mut self) {
        for kept in &mut self.layers {
            kept.fresh = false;
        }
        for (name, view) in &mut self.views {
            let p = view.pointer;
            if p.down && !view.was_down {
                view.pressed = p.at.and_then(|at| {
                    self.layers
                        .iter()
                        .enumerate()
                        .rev()
                        .filter(|(_, k)| &k.layer.view == name)
                        .find_map(|(i, k)| hit(&k.last, at).map(|r| (LayerId(i), r.id.clone())))
                });
            }
        }
    }

    /// Ends a frame: every layer, as drawn now.
    pub fn end_frame(&mut self) -> Vec<Layer> {
        for kept in &mut self.layers {
            kept.last.clone_from(&kept.layer.regions);
        }
        for view in self.views.values_mut() {
            view.was_down = view.pointer.down;
            view.pointer.context = false;
            if !view.pointer.down {
                view.pressed = None;
            }
        }
        self.layers.iter().map(|k| k.layer.clone()).collect()
    }
}

/// The region of `regions` under `p`: the last that holds it.
fn hit(regions: &[Region], p: Point) -> Option<&Region> {
    regions.iter().rev().find(|r| r.rect.contains(p))
}

fn fresh(kept: &mut Kept) {
    if !kept.fresh {
        kept.fresh = true;
        kept.layer.draw.clear();
        kept.layer.regions.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{Layer, Overlays, Plane, Region, ViewPointer};
    use crate::geometry::{Point, Rect};

    #[test]
    fn the_last_region_claimed_takes_the_pointer_and_the_rest_passes_through() {
        let mut layer = Layer::new("markers", "session.arrange", Plane::Over);
        layer.regions.push(Region {
            id: "a".into(),
            rect: Rect::new(0.0, 0.0, 20.0, 20.0),
        });
        layer.regions.push(Region {
            id: "b".into(),
            rect: Rect::new(10.0, 0.0, 20.0, 20.0),
        });
        assert_eq!(
            layer.hit(Point::new(5.0, 5.0)).map(|r| r.id.as_str()),
            Some("a")
        );
        assert_eq!(
            layer.hit(Point::new(15.0, 5.0)).map(|r| r.id.as_str()),
            Some("b")
        );
        assert_eq!(layer.hit(Point::new(50.0, 5.0)), None);
    }

    #[test]
    fn a_region_is_clicked_by_a_press_and_release_over_it() {
        let mut o = Overlays::new();
        let view = "session.arrange";
        let lane = o.layer("lane", view, Plane::Over);
        let flag = Rect::new(100.0, 0.0, 10.0, 16.0);
        let frame = |o: &mut Overlays| {
            o.begin_frame();
            let s = o.region(lane, "m1", flag);
            o.end_frame();
            s
        };
        assert!(!frame(&mut o).hovered, "no pointer yet");
        let over = Some(Point::new(104.0, 8.0));
        o.pointer(
            view,
            ViewPointer {
                at: over,
                down: false,
                context: false,
            },
        );
        assert!(frame(&mut o).hovered);
        o.pointer(
            view,
            ViewPointer {
                at: over,
                down: true,
                context: false,
            },
        );
        let held = frame(&mut o);
        assert!(held.held && !held.clicked);
        o.pointer(
            view,
            ViewPointer {
                at: over,
                down: false,
                context: false,
            },
        );
        assert!(frame(&mut o).clicked);
        assert!(!frame(&mut o).clicked, "a click is one frame's");
        o.pointer(
            view,
            ViewPointer {
                at: over,
                down: false,
                context: true,
            },
        );
        assert!(frame(&mut o).context);
        // Pressed beside it and let go over it is not a click.
        o.pointer(
            view,
            ViewPointer {
                at: Some(Point::new(5.0, 5.0)),
                down: true,
                context: false,
            },
        );
        frame(&mut o);
        o.pointer(
            view,
            ViewPointer {
                at: over,
                down: false,
                context: false,
            },
        );
        assert!(!frame(&mut o).clicked);
    }

    #[test]
    fn a_layer_not_drawn_in_a_frame_keeps_its_drawing() {
        let mut o = Overlays::new();
        let lane = o.layer("lane", "v", Plane::Under);
        o.begin_frame();
        if let Some(d) = o.draw(lane) {
            d.rect_filled(Rect::new(0.0, 0.0, 1.0, 1.0), crate::Color::WHITE, 0.0);
        }
        assert_eq!(
            o.end_frame().first().map(|l| l.draw.shapes().len()),
            Some(1)
        );
        o.begin_frame();
        assert_eq!(
            o.end_frame().first().map(|l| l.draw.shapes().len()),
            Some(1)
        );
        o.begin_frame();
        if let Some(d) = o.draw(lane) {
            d.line(
                Point::new(0.0, 0.0),
                Point::new(1.0, 1.0),
                crate::Color::WHITE,
                1.0,
            );
            d.line(
                Point::new(0.0, 0.0),
                Point::new(1.0, 1.0),
                crate::Color::WHITE,
                1.0,
            );
        }
        assert_eq!(
            o.end_frame().first().map(|l| l.draw.shapes().len()),
            Some(2)
        );
        assert_eq!(o.layer("lane", "v", Plane::Top), lane, "the same layer");
    }
}
