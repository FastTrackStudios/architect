//! List row — horizontal card with leading indicator, text stack, and trailing action.
//!
//! Styled to match shadcn v4 maia card/item patterns.

use architect_story_runtime::story;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct ListRowProps {
    /// Primary label text.
    pub label: String,

    /// Optional secondary detail text below the label.
    #[props(default)]
    pub detail: String,

    /// Optional small tag rendered next to the label.
    #[props(default)]
    pub tag: String,

    /// Optional leading element (status dot, icon, avatar).
    #[props(default)]
    pub leading: Option<Element>,

    /// Optional trailing element (action button, chevron).
    #[props(default)]
    pub trailing: Option<Element>,

    /// Click handler for the entire row.
    #[props(default)]
    pub on_click: Option<Callback<()>>,

    /// Extra CSS classes on the outer container.
    #[props(default)]
    pub class: String,
}

#[component]
pub fn ListRow(props: ListRowProps) -> Element {
    let clickable_class = if props.on_click.is_some() {
        "cursor-pointer hover:bg-accent/30 transition-colors"
    } else {
        ""
    };

    rsx! {
        div {
            class: crate::cn::merge(format!(
                "flex items-center justify-between p-3 rounded-xl border border-border bg-card/50 {} {}",
                clickable_class, props.class
            )),
            onclick: move |_| {
                if let Some(cb) = &props.on_click {
                    cb.call(());
                }
            },

            // Left section: leading + text
            div { class: "flex items-center gap-3 min-w-0",
                if let Some(leading) = &props.leading {
                    {leading}
                }

                div { class: "min-w-0",
                    div { class: "flex items-center gap-2",
                        p { class: "text-sm font-medium text-foreground truncate",
                            "{props.label}"
                        }
                        if !props.tag.is_empty() {
                            span {
                                class: "px-1.5 py-0.5 text-[10px] font-medium rounded-full bg-secondary text-muted-foreground flex-shrink-0",
                                "{props.tag}"
                            }
                        }
                    }
                    if !props.detail.is_empty() {
                        p { class: "text-xs text-muted-foreground truncate",
                            "{props.detail}"
                        }
                    }
                }
            }

            // Right section: trailing
            if let Some(trailing) = &props.trailing {
                div { class: "flex-shrink-0 ml-3",
                    {trailing}
                }
            }
        }
    }
}

#[story(category = "ListRow", name = "list_row_default")]
pub fn list_row_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-col gap-2 w-[28rem]",
            ListRow {
                label: "Signal REAPER".to_string(),
                detail: "Running — PID 1234".to_string(),
                leading: rsx! { crate::components::StatusDot { color: crate::components::StatusDotColor::Success } },
                trailing: rsx! { crate::components::Button { size: crate::components::ButtonSize::Small, variant: crate::components::ButtonVariant::Secondary, "Launch" } },
            }
            ListRow {
                label: "Audio Interface".to_string(),
                detail: "Disconnected".to_string(),
                tag: "USB".to_string(),
                leading: rsx! { crate::components::StatusDot { color: crate::components::StatusDotColor::Danger } },
            }
        }
    }
}
