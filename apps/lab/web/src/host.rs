//! The page: an agent's surface, the confirmation sheet, and the Action
//! List they share.

// Dioxus is single-threaded: its tasks are not `Send`.
#![allow(clippy::future_not_send)]

use std::cell::RefCell;
use std::sync::Arc;

use architect::action::{ActionCaller, ActionList, ActionMeta, CONFIRM_TTL, Effect, Principal};
use architect_a2ui::store::ROOT;
use architect_a2ui::ui::{A2uiSurface, use_a2ui};
use architect_a2ui::{Surfaces, actions};
use architect_commands::ConfirmSheet;
use crdt::CrdtDoc;
use dioxus::prelude::*;
use serde_json::json;

/// What the agent renders on start: more than ten catalog components, and
/// one outside the catalog (drawn as a placeholder, never run).
pub const SURFACE: &str = include_str!("surface.json");

/// Its surface's id.
pub const FORM: &str = "form";

static SAVE: ActionMeta = ActionMeta {
    id: "LAB_SAVE",
    trait_name: "",
    method_name: "",
    display_name: "Save the form",
    description: "Keeps what the form says, and says so on it.",
    category: "Lab",
    group: "Lab",
    shortcut: "",
    toggleable: false,
    undo: false,
};

#[derive(facet::Facet)]
struct Form {
    name: String,
    subscribe: bool,
}

/// The agent: who renders the surface, and whom the page's `a2ui` runs as.
#[must_use]
pub fn agent() -> ActionCaller {
    ActionCaller::new(Principal::Service {
        name: "agent".to_owned(),
    })
}

thread_local! {
    static LIST: RefCell<Option<Arc<ActionList>>> = const { RefCell::new(None) };
}

/// The page's Action List, once the app is up.
#[must_use]
pub fn list() -> Option<Arc<ActionList>> {
    LIST.with(|l| l.borrow().clone())
}

/// The list over `surfaces`: `LAB_SAVE` and the `A2UI_*` actions, with
/// every write by a caller waiting for the person's yes. A failure is
/// said on the page, over whatever was registered.
fn setup(surfaces: &Surfaces) -> (Arc<ActionList>, Option<String>) {
    let list = Arc::new(ActionList::new());
    list.require_confirmation(CONFIRM_TTL);
    let kept = surfaces.clone();
    let made = list
        .register(&SAVE, Effect::Mutating, move |f: Form| {
            let note = if f.subscribe {
                format!("Saved {}, subscribed", f.name)
            } else {
                format!("Saved {}", f.name)
            };
            kept.set_data(FORM, "/saved", Some(&json!(note)))
                .map_err(|e| e.to_string())
        })
        .and_then(|()| actions::register(&list, surfaces))
        .map_err(|e| e.to_string())
        .and_then(|()| list.validate().map_err(|e| e.to_string()));
    (list, made.err())
}

/// The agent renders [`SURFACE`], as it would over MCP.
async fn render(list: &ActionList) -> Result<(), String> {
    list.run_json_async_with(&agent(), actions::RENDER, SURFACE.as_bytes())
        .await
        .map(drop)
        .map_err(|e| e.to_string())
}

#[component]
pub fn App() -> Element {
    let surfaces = use_hook(|| Surfaces::new(CrdtDoc::ephemeral(), ROOT));
    let (list, broken) = use_hook(|| setup(&surfaces));
    use_context_provider(|| list.clone());
    use_hook(|| LIST.with(|l| *l.borrow_mut() = Some(list.clone())));
    use_a2ui(surfaces, (*list).clone());
    let mut failed = use_signal(|| broken);
    use_future(move || {
        let list = list.clone();
        async move {
            if let Err(why) = render(&list).await {
                failed.set(Some(why));
            }
        }
    });
    rsx! {
        div { "data-lab-web": "", class: "relative min-h-screen bg-slate-100 text-slate-900",
            header { "data-app-header": "", class: "bg-indigo-700 px-6 py-3 text-white shadow",
                h1 { class: "text-lg font-semibold tracking-tight", "architect lab · web" }
                p { class: "text-sm text-indigo-100", "An agent's UI, drawn with architect-ui in the browser." }
            }
            main { class: "mx-auto max-w-2xl p-6",
                if let Some(why) = failed() {
                    p { "data-lab-error": "", class: "mb-4 text-red-700", "{why}" }
                }
                div { class: "rounded-xl bg-white shadow-sm ring-1 ring-slate-200",
                    A2uiSurface { id: FORM }
                }
            }
            ConfirmSheet {}
        }
    }
}

#[cfg(test)]
mod tests {
    use architect_a2ui::events::{self, Outcome};
    use serde_json::Value;

    use super::*;

    #[test]
    fn the_agent_renders_more_than_ten_components_and_a_save_waits() {
        let surfaces = Surfaces::new(CrdtDoc::ephemeral(), ROOT);
        let (list, broken) = setup(&surfaces);
        assert_eq!(broken, None);
        let surface: Value = serde_json::from_str(SURFACE).unwrap();
        assert_eq!(
            surface["messages"][0]["createSurface"]["catalogId"],
            architect_a2ui::CATALOG_ID
        );

        let out = futures::executor::block_on(list.run_json_async_with(
            &agent(),
            actions::RENDER,
            SURFACE.as_bytes(),
        ))
        .unwrap();
        let out: Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(out["surfaces"], json!([FORM]));
        assert!(out["components"].as_u64().unwrap() > 10, "{out}");
        assert_eq!(out["outsideCatalog"].as_array().unwrap().len(), 1);
        assert_eq!(out["outsideCatalog"][0]["component"], "Script");

        let press =
            futures::executor::block_on(events::press(&list, &surfaces, (FORM, "save", ""), None));
        assert!(
            matches!(press, Outcome::Waiting { ref id, .. } if id == "LAB_SAVE"),
            "{press:?}"
        );
        assert_eq!(surfaces.get(FORM).unwrap().data["saved"], "Not saved yet");
    }
}
