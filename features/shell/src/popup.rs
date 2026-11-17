//! A menu a view asks the window to show (feature `ui`).
//!
//! Drawn over everything, at the pointer, kept inside the window and
//! closed by a press anywhere else. A view drawn in a pane cannot do that
//! itself: Blitz has no `position: fixed`, so what it draws stays in its
//! pane.
//!
//! The workspace host provides it (`architect_workspace`'s menu layer
//! draws the rows); where nothing does, [`try_use_popup`] is `None` and
//! the view draws its own menu.

use std::fmt;
use std::rc::Rc;

use dioxus::prelude::*;

/// What a row does when pressed.
pub type PopupRun = Rc<dyn Fn()>;

/// One of a popup menu's rows.
#[derive(Clone)]
pub enum PopupRow {
    /// A section's name.
    Heading(String),
    /// A rule between sections.
    Separator,
    /// A row that does something; `danger` draws it in the danger ink
    /// (it deletes); `checked` keeps a column for a tick (one of a
    /// choice, or a toggle), ticked when `Some(true)`.
    Act {
        label: String,
        danger: bool,
        checked: Option<bool>,
        run: PopupRun,
    },
    /// A line that does nothing (a menu with no rows says so).
    Note(String),
}

impl PopupRow {
    /// A row that runs `run`.
    pub fn act(label: impl Into<String>, run: impl Fn() + 'static) -> Self {
        Self::Act {
            label: label.into(),
            danger: false,
            checked: None,
            run: Rc::new(run),
        }
    }

    /// A row that deletes something, drawn in the danger ink.
    pub fn danger(label: impl Into<String>, run: impl Fn() + 'static) -> Self {
        Self::Act {
            label: label.into(),
            danger: true,
            checked: None,
            run: Rc::new(run),
        }
    }

    /// One of a choice (or a toggle): ticked when `on`.
    pub fn check(label: impl Into<String>, on: bool, run: impl Fn() + 'static) -> Self {
        Self::Act {
            label: label.into(),
            danger: false,
            checked: Some(on),
            run: Rc::new(run),
        }
    }
}

impl PartialEq for PopupRow {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Heading(a), Self::Heading(b)) | (Self::Note(a), Self::Note(b)) => a == b,
            (Self::Separator, Self::Separator) => true,
            (
                Self::Act {
                    label: a,
                    danger: da,
                    checked: ca,
                    run: ra,
                },
                Self::Act {
                    label: b,
                    danger: db,
                    checked: cb,
                    run: rb,
                },
            ) => a == b && da == db && ca == cb && Rc::ptr_eq(ra, rb),
            _ => false,
        }
    }
}

impl fmt::Debug for PopupRow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Heading(t) => f.debug_tuple("Heading").field(t).finish(),
            Self::Separator => f.write_str("Separator"),
            Self::Act {
                label,
                danger,
                checked,
                ..
            } => f
                .debug_struct("Act")
                .field("label", label)
                .field("danger", danger)
                .field("checked", checked)
                .finish_non_exhaustive(),
            Self::Note(t) => f.debug_tuple("Note").field(t).finish(),
        }
    }
}

/// Shows a popup menu: its rows, at a point in window coordinates (a
/// `MouseEvent`'s `client_coordinates`).
#[derive(Clone)]
pub struct Popup {
    show: Rc<dyn Fn(Vec<PopupRow>, (f64, f64))>,
}

impl Popup {
    /// The provider's: `show` draws the rows at the point.
    pub fn new(show: impl Fn(Vec<PopupRow>, (f64, f64)) + 'static) -> Self {
        Self {
            show: Rc::new(show),
        }
    }

    /// Shows `rows` at `at` (window points), over everything.
    pub fn show(&self, rows: Vec<PopupRow>, at: (f64, f64)) {
        (self.show)(rows, at);
    }
}

/// Where a menu opened from a button goes: under the button `e` pressed
/// (its left edge, its foot), in window points.
#[must_use]
pub fn under(e: &MouseEvent, height: f64) -> (f64, f64) {
    let c = e.client_coordinates();
    let at = e.element_coordinates();
    (c.x - at.x, c.y - at.y + height + 4.0)
}

/// The window's popup menus, where something provides them (the
/// workspace host); `None` elsewhere. Also callable in an event handler.
#[must_use]
pub fn try_use_popup() -> Option<Popup> {
    try_consume_context::<Popup>()
}
