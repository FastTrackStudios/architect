//! Spacer — flex-grow element that fills available space.

use dioxus::prelude::*;

/// Fills remaining flex space. Useful for pushing items to opposite ends.
#[component]
pub fn Spacer() -> Element {
    rsx! {
        div { class: "flex-1" }
    }
}
