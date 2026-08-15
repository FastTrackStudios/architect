//! NavTab — standalone navigation pill button.
//!
//! A pill-shaped button with active/inactive styling for primary navigation bars.
//! Unlike `Tabs` (context-based content switcher) or `SegmentedControl` (grouped toggle),
//! NavTab is a standalone button meant for top-level route navigation.

use dioxus::prelude::*;
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct NavTabProps {
    /// Display text.
    pub label: String,
    /// Identifier passed to callback on click.
    pub tab_id: String,
    /// Whether this tab is currently active.
    pub is_active: bool,
    /// Click handler — receives `tab_id`.
    #[props(default)]
    pub on_click: Option<Callback<String>>,
    /// Extra CSS classes.
    #[props(default)]
    pub class: String,
}

/// Standalone navigation pill button.
#[component]
pub fn NavTab(props: NavTabProps) -> Element {
    let active_class = if props.is_active {
        "bg-primary text-primary-foreground"
    } else {
        "text-muted-foreground hover:text-foreground hover:bg-accent"
    };

    rsx! {
        button {
            class: crate::cn::merge_slice(&["px-4 py-2 rounded-md font-medium text-sm transition-colors", active_class, props.class.as_str()]),
            onclick: {
                let on_click = props.on_click;
                let tab_id = props.tab_id.clone();
                move |_| {
                    if let Some(callback) = &on_click {
                        callback.call(tab_id.clone());
                    }
                }
            },
            "{props.label}"
        }
    }
}

/// Row of NavTab pills with one active.
#[story(category = "NavTab", name = "default")]
pub fn nav_tab_default() -> Element {
    let mut active = use_signal(|| "overview".to_string());
    let on_click = use_callback(move |id: String| active.set(id));

    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center gap-2",
            NavTab {
                label: "Overview".to_string(),
                tab_id: "overview".to_string(),
                is_active: active() == "overview",
                on_click,
            }
            NavTab {
                label: "Activity".to_string(),
                tab_id: "activity".to_string(),
                is_active: active() == "activity",
                on_click,
            }
            NavTab {
                label: "Reports".to_string(),
                tab_id: "reports".to_string(),
                is_active: active() == "reports",
                on_click,
            }
        }
    }
}
