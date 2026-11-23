//! Another extension's view held in this one (`architect_extensions::embedded`):
//! a placeholder naming the view and its input (`VIEW_ATTR`, `INPUT_ATTR`
//! there), which the host draws over.

use architect_extensions::contrib::EmbedInput;
use dioxus::prelude::*;

/// Another extension's view, held here: `extension`'s view `view`,
/// given `input`, filling this element (sized as `style` says; it
/// fills its parent by default). The host draws it.
#[component]
pub fn EmbeddedView(
    extension: String,
    view: String,
    #[props(default)] input: EmbedInput,
    #[props(default)] style: String,
) -> Element {
    let name = format!("{extension}/{view}");
    let input = serde_json::to_string(&input).unwrap_or_default();
    rsx! {
        div {
            style: "display: block; width: 100%; height: 100%; {style}",
            "data-architect-embed-view": "{name}",
            "data-architect-embed-input": "{input}",
        }
    }
}
