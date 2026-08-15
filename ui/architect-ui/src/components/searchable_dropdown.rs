//! Searchable dropdown — floating popover with backdrop, search input, and
//! scrollable children.
//!
//! Positioned with `position: fixed` at explicit `(x, y)` coordinates so it
//! escapes CSS `transform` stacking contexts (important for Wry/WebKit).

use std::sync::atomic::{AtomicU64, Ordering};

use dioxus::prelude::*;
use architect_story_runtime::story;

static DROPDOWN_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

// ---------------------------------------------------------------------------
// SearchableDropdown
// ---------------------------------------------------------------------------

/// Floating searchable dropdown panel with a click-away backdrop.
#[derive(Props, Clone, PartialEq)]
pub struct SearchableDropdownProps {
    /// Whether the dropdown is visible.
    pub open: bool,

    /// Called when the backdrop is clicked or Escape is pressed.
    pub on_close: Callback<()>,

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

    /// Fixed-position X coordinate (pixels).
    #[props(default = 0.0)]
    pub x: f64,

    /// Fixed-position Y coordinate (pixels).
    #[props(default = 0.0)]
    pub y: f64,

    /// Width class (e.g. `"w-60"`).
    #[props(default = "w-60".to_string())]
    pub width: String,

    /// Max-height class (e.g. `"max-h-80"`).
    #[props(default = "max-h-80".to_string())]
    pub max_height: String,

    /// Optional header slot rendered above the search input (for tabs/pills).
    #[props(default)]
    pub header: Option<Element>,

    /// Extra CSS classes on the panel.
    #[props(default)]
    pub class: String,

    /// The filtered results to display.
    pub children: Element,
}

#[component]
pub fn SearchableDropdown(props: SearchableDropdownProps) -> Element {
    if !props.open {
        return rsx! {};
    }

    let mut value = props.value;
    let input_id = use_signal(|| {
        let id = DROPDOWN_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        format!("searchable-dropdown-{id}")
    });
    let iid = input_id();

    let panel_style = format!(
        "position: fixed; left: {}px; top: {}px; z-index: 9999;",
        props.x, props.y
    );

    // Auto-focus the search input on mount.
    let focus_js = format!(
        r#"(function(){{ var el = document.getElementById('{iid}'); if(el) el.focus(); }})()"#
    );

    rsx! {
        // Backdrop
        div {
            class: "fixed inset-0 bg-black/80",
            style: "z-index: 9998;",
            onmousedown: move |evt| {
                evt.stop_propagation();
                props.on_close.call(());
            },
        }

        // Panel
        div {
            class: crate::cn::merge(format!(
                "{} {} bg-background border border-border rounded-xl shadow-md shadow-black/50 \
                 flex flex-col overflow-hidden {}",
                props.width,
                props.max_height,
                props.class
            )),
            style: "{panel_style}",
            onclick: move |evt| evt.stop_propagation(),
            onkeydown: move |evt| {
                if evt.key() == Key::Escape {
                    props.on_close.call(());
                }
                evt.stop_propagation();
            },

            // Optional header (tabs, pills, etc.)
            if let Some(header) = &props.header {
                {header}
            }

            // Search input
            div { class: "px-3 py-1.5 border-b border-border",
                input {
                    id: "{iid}",
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
            script { "{focus_js}" }

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

#[story(category = "SearchableDropdown", name = "searchable_dropdown_default")]
pub fn searchable_dropdown_default() -> Element {
    let value = use_signal(String::new);
    let mut open = use_signal(|| false);

    let items = vec!["Apple", "Banana", "Cherry", "Durian", "Elderberry", "Fig"];
    let query = value().to_lowercase();
    let filtered: Vec<&'static str> = items
        .iter()
        .copied()
        .filter(|i| query.is_empty() || i.to_lowercase().contains(&query))
        .collect();

    rsx! {
        div { class: "p-6 bg-background text-foreground relative h-96",
            button {
                class: "h-9 px-3 rounded-lg border border-border text-sm",
                onclick: move |_| open.set(true),
                if open() { "Dropdown open…" } else { "Open searchable dropdown" }
            }
            SearchableDropdown {
                open: open(),
                on_close: move |_| open.set(false),
                value,
                placeholder: "Search fruit...".to_string(),
                has_results: !filtered.is_empty(),
                // Anchor near the trigger. The Lookbook content area
                // starts ~240px right of the page (sidebar) and ~48px
                // below the top bar, plus the story padding (p-6).
                x: 280.0,
                y: 110.0,
                for item in filtered {
                    div {
                        key: "{item}",
                        class: "px-2 py-1.5 text-xs text-foreground hover:bg-accent rounded cursor-pointer",
                        onclick: move |_| open.set(false),
                        "{item}"
                    }
                }
            }
        }
    }
}
