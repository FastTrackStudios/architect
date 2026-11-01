//! The lab as a guide scenario's app (`architect-guide`): its root,
//! sized to the scenario's window, a fresh Action List, and its fonts.

use std::sync::Arc;

use architect::action::ActionList;
use architect_guide::ViewSize;
use architect_guide::run::App;
use architect_lab_ui::{Device, LabApp};
use architect_shell::{Insets, Screen};
use dioxus::prelude::*;

/// Where the lab's scenarios are, from the workspace's root.
pub const SCENARIOS: &str = "apps/lab/scenarios";

/// The lab, for one run.
#[must_use]
pub fn app() -> App {
    App::new(Root, Arc::new(ActionList::new())).fonts(architect_lab_ui::font_context())
}

/// What the lab's desk window gives it (`apps/lab/desktop`), headless: the
/// screen it is on, the scenario's window, a desk.
#[component]
fn Root() -> Element {
    let view = use_context::<ViewSize>();
    use_context_provider(|| Signal::new(Device::Native));
    use_context_provider(|| {
        Signal::new(Screen {
            width: f64::from(view.width),
            height: f64::from(view.height),
            touch: false,
            insets: Insets::default(),
        })
    });
    rsx! { LabApp {} }
}
