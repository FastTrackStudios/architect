//! Extensions' `process` views (#838) in a Dioxus window: what draws one.

use std::rc::Rc;

pub use architect_extensions::process_view::ProcessAsk;
use dioxus::prelude::Element;

/// Draws extensions' `process` views (#838).
///
/// A program in the package, its GPU frames in the pane
/// (`architect-process-view`'s `extension::use_process_views`, which checks
/// each with `architect_extensions::process_view::launch`). An app with no
/// process-view host provides none, and such a view says why in its pane.
#[derive(Clone)]
pub struct ProcessViews(pub Rc<dyn Fn(&ProcessAsk) -> Element>);
