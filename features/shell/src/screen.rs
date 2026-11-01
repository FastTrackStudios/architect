//! The screen the shell lays out for: its size, its form factor, the
//! insets it keeps clear, and the style an app picks. Pure data — the
//! crate's contract, with no Dioxus.

/// What sort of screen the window is: which layout the shell takes.
/// Phone, tablet and desk are sizes, not devices.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FormFactor {
    /// Upright and narrower than [`Self::NARROW`], or on its side and
    /// shorter than [`Self::SHORT`].
    Phone,
    /// Anything larger, under a finger.
    Tablet,
    /// Anything larger, under a mouse.
    Desk,
}

impl FormFactor {
    /// Upright and narrower than this is a phone's.
    pub const NARROW: f64 = 700.0;
    /// On its side and shorter than this is a phone's.
    pub const SHORT: f64 = 500.0;

    /// The form of a window `width` × `height` points, `touch` where the
    /// screen is the pointer.
    #[must_use]
    pub fn of(width: f64, height: f64, touch: bool) -> Self {
        let phone = if width <= height {
            width < Self::NARROW
        } else {
            height < Self::SHORT
        };
        if phone {
            Self::Phone
        } else if touch {
            Self::Tablet
        } else {
            Self::Desk
        }
    }
}

/// How far the page keeps clear of each edge itself (the status bar, the
/// camera housing, the home indicator), in points.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Insets {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

/// The window as the shell lays out for it, from the app's host.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Screen {
    pub width: f64,
    pub height: f64,
    /// Whether the screen is the pointer.
    pub touch: bool,
    pub insets: Insets,
}

impl Screen {
    #[must_use]
    pub fn form(&self) -> FormFactor {
        FormFactor::of(self.width, self.height, self.touch)
    }

    #[must_use]
    pub fn landscape(&self) -> bool {
        self.width > self.height
    }
}

/// One look of the shell. A style changes how the slots look, never which
/// slots there are.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum ShellStyle {
    /// Task's: the tabs in a floating pill, the separate button round
    /// beside it, the ⋯ pill at the top.
    #[default]
    FloatingPill,
    /// The iOS tab bar: full width, labels under the icons, the separate
    /// button set apart at the right end.
    TabBar,
    /// No bottom bar on any size: a rail on the left with the separate
    /// button at its foot.
    RailOnly,
}
