//! Searchable list panel — inline search input + scrollable children.
//!
//! The consumer owns filtering: pass a `Signal<String>` for the search text,
//! filter your data, and render the results as `children`. The component
//! provides the search input, scrollable container, and empty-state fallback.

use architect_story_runtime::story;
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// SearchableList
// ---------------------------------------------------------------------------

/// Inline searchable panel with a search input and scrollable item area.
#[derive(Props, Clone, PartialEq)]
pub struct SearchableListProps {
    /// Two-way bound search text (consumer owns this for filtering).
    pub value: Signal<String>,

    /// Placeholder text for the search input.
    #[props(default = "Search...".to_string())]
    pub placeholder: String,

    /// Text shown when there are no results.
    #[props(default = "No results".to_string())]
    pub empty_label: String,

    /// Whether the children contain any results. Controls empty-state display.
    #[props(default = true)]
    pub has_results: bool,

    /// Extra CSS classes on the outer container.
    #[props(default)]
    pub class: String,

    /// The filtered items to display.
    pub children: Element,
}

#[component]
pub fn SearchableList(props: SearchableListProps) -> Element {
    let mut value = props.value;

    rsx! {
        div {
            class: crate::cn::merge(format!(
                "flex flex-col bg-background border border-border rounded-xl overflow-hidden {}",
                props.class
            )),

            // Search input
            div { class: "px-3 py-1.5 border-b border-border",
                input {
                    class: "w-full bg-input/30 border border-border rounded-md px-2.5 py-1.5 \
                            text-[11px] text-foreground outline-none focus:border-ring \
                            placeholder:text-muted-foreground transition-all",
                    r#type: "text",
                    placeholder: "{props.placeholder}",
                    value: "{value}",
                    oninput: move |evt| value.set(evt.value().clone()),
                    autofocus: true,
                    onmounted: move |elem| async move {
                        let _ = elem.set_focus(true).await;
                    },
                }
            }

            // Scrollable results
            div { class: "flex-1 overflow-y-auto min-h-0 px-1.5 py-1.5",
                if !props.has_results {
                    div { class: "flex items-center justify-center py-4",
                        p { class: "text-[10px] text-muted-foreground", "{props.empty_label}" }
                    }
                } else {
                    {props.children}
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SearchableListItem
// ---------------------------------------------------------------------------

/// A clickable row for use inside `SearchableList` or `SearchableDropdown`.
///
/// Renders an optional leading icon, a label, and an optional description.
#[derive(Props, Clone, PartialEq)]
pub struct SearchableListItemProps {
    /// Click handler.
    pub on_click: Callback<()>,

    /// Primary label text.
    pub label: String,

    /// Optional leading element (icon, colored dot, badge).
    #[props(default)]
    pub icon: Option<Element>,

    /// Optional secondary description text.
    #[props(default)]
    pub description: String,

    /// Whether this item is visually selected/highlighted.
    #[props(default = false)]
    pub selected: bool,

    /// Extra CSS classes.
    #[props(default)]
    pub class: String,
}

#[component]
pub fn SearchableListItem(props: SearchableListItemProps) -> Element {
    let bg_class = if props.selected {
        "bg-accent"
    } else {
        "hover:bg-accent/50"
    };

    rsx! {
        button {
            class: crate::cn::merge(format!(
                "w-full flex items-center gap-2 px-2 py-1.5 rounded-md text-left \
                 transition-all duration-100 {bg_class} {}",
                props.class
            )),
            onclick: move |_| props.on_click.call(()),

            if let Some(icon) = &props.icon {
                {icon}
            }

            div { class: "flex-1 min-w-0",
                span { class: "text-[11px] font-medium text-foreground block truncate",
                    "{props.label}"
                }
                if !props.description.is_empty() {
                    span { class: "text-[9px] text-muted-foreground",
                        "{props.description}"
                    }
                }
            }
        }
    }
}

#[story(category = "SearchableList", name = "searchable_list_default")]
pub fn searchable_list_default() -> Element {
    let value = use_signal(String::new);
    let items = vec!["Apple", "Banana", "Cherry", "Durian"];
    let filter = value().to_lowercase();
    let filtered: Vec<&&str> = items
        .iter()
        .filter(|s| s.to_lowercase().contains(&filter))
        .collect();
    let has_results = !filtered.is_empty();

    rsx! {
        div { class: "p-6 bg-background text-foreground w-80 h-72",
            SearchableList {
                value,
                placeholder: "Filter fruit...".to_string(),
                has_results,
                for item in filtered {
                    SearchableListItem {
                        on_click: move |_| {},
                        label: item.to_string(),
                    }
                }
            }
        }
    }
}
