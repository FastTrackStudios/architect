//! An extension's view as an overlay layer over another view (#175):
//! `[[overlays]] draws = "<one of its views>"`.
//!
//! The view a layer draws over (the arrangement, the editor) publishes its
//! coordinate spaces — its size, time across, lanes down, already scrolled
//! — as [`OverlaySpaces`]. The host mounts the extension's view over that
//! view's box, on its plane, clipped to it and passing the pointer
//! through, and hands it the spaces whenever they change, as an embed's
//! input: its `props` (`claim` is [`claim`]). The extension reads them with
//! `architect_extensions_dioxus::overlay::spaces` (its embed input) and draws in the view's terms, a
//! `scene::RemoteScene` filling its box, so its element's coordinates are
//! the view's.
//!
//! A Luau overlay (`script`) draws through `architect-canvas`'s
//! `Overlays` instead; both are given the same spaces.

use serde::{Deserialize, Serialize};

/// A view's coordinate spaces, as an overlay is given them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OverlaySpaces {
    /// The view's visible size, points.
    pub width: f32,
    pub height: f32,
    /// Time across, for a view that has it.
    pub time: Option<TimeAxis>,
    /// Its lanes, top to bottom, scrolled.
    pub lanes: Vec<Lane>,
}

/// Time across a view: the time at its left edge and its zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TimeAxis {
    /// Seconds at x = 0 (scrolled).
    pub start: f64,
    /// Points per second; above zero.
    pub pixels_per_second: f64,
}

impl TimeAxis {
    /// Where `seconds` is across the view.
    #[must_use]
    pub fn x(&self, seconds: f64) -> f64 {
        (seconds - self.start) * self.pixels_per_second
    }
}

/// One lane down a view (a track), scrolled, points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Lane {
    pub id: String,
    pub name: String,
    pub top: f32,
    pub height: f32,
}

impl OverlaySpaces {
    /// Lane `id`, if the view has it.
    #[must_use]
    pub fn lane(&self, id: &str) -> Option<&Lane> {
        self.lanes.iter().find(|l| l.id == id)
    }
}

/// The embed claim an overlay over views of kind `view` is given its
/// spaces under.
#[must_use]
pub fn claim(view: &str) -> String {
    format!("overlay:{view}")
}

/// The spaces of the view an overlay draws over, from its embed input's
/// `props`; `None` when it isn't one (or shows in a pane of its own).
#[must_use]
pub fn from_input(input: &crate::contrib::EmbedInput) -> Option<OverlaySpaces> {
    if !input.claim.starts_with("overlay:") {
        return None;
    }
    serde_json::from_value(input.props.clone()).ok()
}

/// One extension's view drawn as a layer over views of a kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewOverlay {
    /// The extension.
    pub extension: String,
    /// Its overlay's id.
    pub id: String,
    /// The view of its own that draws it.
    pub draws: String,
    /// `under`, `over` or `top`.
    pub plane: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_reach_an_overlay_as_its_input_and_nothing_else_does() {
        let spaces = OverlaySpaces {
            width: 400.0,
            height: 300.0,
            time: Some(TimeAxis {
                start: 2.0,
                pixels_per_second: 50.0,
            }),
            lanes: vec![Lane {
                id: "kick".into(),
                name: "Kick".into(),
                top: 40.0,
                height: 30.0,
            }],
        };
        let input = crate::contrib::EmbedInput {
            claim: claim("lab.arrange"),
            props: serde_json::to_value(&spaces).unwrap_or_default(),
            ..crate::contrib::EmbedInput::default()
        };
        assert_eq!(from_input(&input), Some(spaces.clone()));
        assert!((spaces.time.unwrap().x(4.0) - 100.0).abs() < 1e-9);
        assert_eq!(spaces.lane("kick").map(|l| l.top), Some(40.0));
        let fence = crate::contrib::EmbedInput {
            claim: "fence:kf".into(),
            ..crate::contrib::EmbedInput::default()
        };
        assert_eq!(from_input(&fence), None);
    }
}
