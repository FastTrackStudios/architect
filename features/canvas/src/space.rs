//! A view's coordinate spaces: what it publishes so a layer can draw in
//! its terms.
//!
//! Time across (the arrangement's visible range and zoom), lanes down
//! (each track's top and height) — already scrolled, in the view's points.
//!
//! A view publishes them when they change (a scroll, a zoom, a track
//! resized), not every frame; a layer drawn in them is redrawn then.

use crate::geometry::Point;

/// Time across a view: the time at its left edge and its zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeAxis {
    /// Seconds at x = 0 (scrolled).
    pub start: f64,
    /// Points per second (the zoom); above zero.
    pub pixels_per_second: f64,
}

impl TimeAxis {
    /// Where `seconds` is across the view.
    #[must_use]
    pub fn x(&self, seconds: f64) -> f32 {
        // Points on a screen: far inside f32's range.
        #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
        let x = ((seconds - self.start) * self.pixels_per_second) as f32;
        x
    }

    /// The time at `x` across the view.
    #[must_use]
    pub fn time(&self, x: f32) -> f64 {
        f64::from(x).mul_add(1.0 / self.pixels_per_second.max(f64::EPSILON), self.start)
    }
}

/// One lane down a view: a track, a staff, a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Lane {
    /// Stable (a track's id).
    pub id: String,
    /// What a person calls it ("Kick").
    pub name: String,
    /// Its top, scrolled, in the view's points: above zero once scrolled
    /// past; a pinned lane stays where it is pinned.
    pub top: f32,
    pub height: f32,
}

impl Lane {
    pub fn new(id: impl Into<String>, name: impl Into<String>, top: f32, height: f32) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            top,
            height,
        }
    }
}

/// Everything a view publishes of its coordinates.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spaces {
    /// Its visible size, in points.
    pub width: f32,
    pub height: f32,
    /// Time across, for a view that has it.
    pub time: Option<TimeAxis>,
    /// Its lanes, top to bottom (hidden ones left out).
    pub lanes: Vec<Lane>,
}

impl Spaces {
    /// A view `width` by `height` with no coordinates of its own.
    #[must_use]
    pub const fn sized(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            time: None,
            lanes: Vec::new(),
        }
    }

    #[must_use]
    pub const fn with_time(mut self, time: TimeAxis) -> Self {
        self.time = Some(time);
        self
    }

    #[must_use]
    pub fn with_lane(mut self, lane: Lane) -> Self {
        self.lanes.push(lane);
        self
    }

    /// The lane whose id — or else whose name — is `key`.
    #[must_use]
    pub fn lane(&self, key: &str) -> Option<&Lane> {
        self.lanes
            .iter()
            .find(|l| l.id == key)
            .or_else(|| self.lanes.iter().find(|l| l.name == key))
    }

    /// The lane at `y`.
    #[must_use]
    pub fn lane_at(&self, y: f32) -> Option<&Lane> {
        self.lanes
            .iter()
            .find(|l| y >= l.top && y < l.top + l.height)
    }

    /// Where `{ time, lane }` is in the view: `x` from the time (0 with
    /// none), `y` the lane's top (0 with none). `None` when it names a
    /// time the view has no axis for, or a lane it does not have.
    #[must_use]
    pub fn at(&self, time: Option<f64>, lane: Option<&str>) -> Option<Point> {
        let x = match time {
            Some(t) => self.time?.x(t),
            None => 0.0,
        };
        let y = match lane {
            Some(key) => self.lane(key)?.top,
            None => 0.0,
        };
        Some(Point::new(x, y))
    }
}

#[cfg(test)]
mod tests {
    use super::{Lane, Spaces, TimeAxis};
    use crate::geometry::Point;

    fn arrangement() -> Spaces {
        Spaces::sized(800.0, 300.0)
            .with_time(TimeAxis {
                start: 10.0,
                pixels_per_second: 50.0,
            })
            .with_lane(Lane::new("t1", "Kick", 0.0, 60.0))
            .with_lane(Lane::new("t2", "Snare", 60.0, 40.0))
    }

    #[test]
    fn time_and_lanes_place_a_point() {
        let s = arrangement();
        assert!((s.time.map_or(0.0, |t| t.x(12.5)) - 125.0).abs() < 1e-3);
        assert!((s.time.map_or(0.0, |t| t.time(125.0)) - 12.5).abs() < 1e-9);
        assert_eq!(
            s.at(Some(12.5), Some("Snare")),
            Some(Point::new(125.0, 60.0))
        );
        assert_eq!(s.at(None, Some("t1")), Some(Point::new(0.0, 0.0)));
        assert_eq!(s.at(None, Some("Bass")), None);
        assert_eq!(Spaces::sized(1.0, 1.0).at(Some(1.0), None), None);
        assert_eq!(s.lane_at(70.0).map(|l| l.name.as_str()), Some("Snare"));
        assert_eq!(s.lane_at(100.0), None);
    }
}
