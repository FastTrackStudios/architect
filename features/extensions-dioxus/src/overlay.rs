//! An overlay's view (`architect_extensions::overlay`): the spaces of the
//! view it draws over.

use architect_extensions::overlay::{OverlaySpaces, from_input};

/// In an overlay's view: the spaces of the view it draws over, now; the
/// view renders again when they change.
#[must_use]
pub fn spaces() -> Option<OverlaySpaces> {
    architect_extensions::linked::use_embed().and_then(|i| from_input(&i))
}
