//! What a pane draws of the others: their pointers, their selections
//! washed in their colour, their playheads — already placed in the pane's
//! points (the view knows its coordinate space; `crdt::presence::Roster`
//! places a remote pointer with the view's own function). Plain data; the
//! host's `ui` feature draws it (`PresenceLayer`).

/// A person's pointer: a dot in their colour with their name.
#[derive(Debug, Clone, PartialEq)]
pub struct PointerMark {
    pub peer: String,
    pub name: String,
    /// `0xRRGGBB`.
    pub color: u32,
    /// From the pane's top left, in points.
    pub x: f64,
    pub y: f64,
}

/// A person's selection: a wash in their colour over a box (pane points).
#[derive(Debug, Clone, PartialEq)]
pub struct WashMark {
    pub peer: String,
    pub color: u32,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// A person's playhead: a line in their colour at `x` (pane points).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayheadMark {
    pub peer: String,
    pub color: u32,
    pub x: f64,
}

/// Everything a pane draws of the others.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PresenceMarks {
    pub pointers: Vec<PointerMark>,
    pub washes: Vec<WashMark>,
    pub playheads: Vec<PlayheadMark>,
}

impl PresenceMarks {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.pointers.is_empty() && self.washes.is_empty() && self.playheads.is_empty()
    }
}

/// `0xRRGGBB` as CSS `rgba(…)` at `alpha` (0..1).
#[must_use]
pub fn css_color(color: u32, alpha: f64) -> String {
    let [_, r, g, b] = color.to_be_bytes();
    format!("rgba({r},{g},{b},{alpha})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_its_three_bytes() {
        assert_eq!(css_color(0x12_34_56, 0.5), "rgba(18,52,86,0.5)");
    }

    #[test]
    fn no_marks_is_empty() {
        assert!(PresenceMarks::default().is_empty());
        let marks = PresenceMarks {
            playheads: vec![PlayheadMark {
                peer: "ann".into(),
                color: 0,
                x: 1.0,
            }],
            ..PresenceMarks::default()
        };
        assert!(!marks.is_empty());
    }
}
