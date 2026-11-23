//! A pane's id, as the workspace that arranges panes and the views drawn
//! in them both name it, and what a pane tells its view ([`PaneHost`]).

use facet::Facet;

/// A pane's id: stable and addressable.
///
/// The one its maker gave it (a script's `"timer"`, a plugin window's
/// `"fx:<track>/<slot>"`), or its view and subject (`"studio.song:<id>"`).
/// Opening an id that is open brings that pane forward instead of making
/// another.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Facet)]
pub struct PaneId(pub String);

impl PaneId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// What the shell tells a view drawn in one of its panes (put in context
/// around the pane as a [`PaneHostRef`]): which pane, its state and where
/// its view is. Read in a render, the methods that read the pane subscribe
/// the component to it, as the shell's own state does.
pub trait PaneHost {
    /// The pane's id.
    fn id(&self) -> PaneId;
    /// Its state (an opener's path; "" for none).
    fn state(&self) -> String;
    /// Where its view is in what it shows, as kept ("" when nothing was).
    fn view_state(&self) -> String;
    /// Keeps `value` as where its view is.
    fn set_view_state(&self, value: String);
}

/// The [`PaneHost`] of the pane a view is drawn in, in context.
#[derive(Clone)]
pub struct PaneHostRef(pub std::rc::Rc<dyn PaneHost>);
