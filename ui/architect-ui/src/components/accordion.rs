//! Accordion — shadcn v4 maia style collapsible sections (single-open mode).

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::accordion::{
    Accordion as PrimitiveAccordion, AccordionContent as PrimitiveAccordionContent,
    AccordionItem as PrimitiveAccordionItem, AccordionTrigger as PrimitiveAccordionTrigger,
};

// ---------------------------------------------------------------------------
// Accordion (container)
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct AccordionProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default = false)]
    pub allow_multiple_open: bool,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = true)]
    pub collapsible: bool,
    #[props(default = false)]
    pub horizontal: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Root accordion container. Provides single-open context to children.
#[component]
pub fn Accordion(props: AccordionProps) -> Element {
    rsx! {
        document::Style {
            r#"
                .fts-accordion-item[data-open="true"] {{
                    background: color-mix(in oklab, var(--muted) 50%, transparent);
                }}

                .fts-accordion-item:not(:last-child) {{
                    border-bottom: 1px solid var(--border);
                }}
            "#
        }
        PrimitiveAccordion {
            id: props.id,
            allow_multiple_open: props.allow_multiple_open,
            disabled: props.disabled,
            collapsible: props.collapsible,
            horizontal: props.horizontal,
            class: crate::cn::merge_slice(
                &["overflow-hidden rounded-lg border border-border bg-card text-card-foreground shadow-sm", props.class.as_str()],
            ),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// AccordionItem
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct AccordionItemProps {
    pub index: usize,
    #[props(default = false)]
    pub default_open: bool,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub on_change: Option<Callback<bool>>,
    #[props(default)]
    pub on_trigger_click: Option<Callback>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Individual collapsible section inside an `Accordion`.
#[component]
pub fn AccordionItem(props: AccordionItemProps) -> Element {
    rsx! {
        PrimitiveAccordionItem {
            index: props.index,
            default_open: props.default_open,
            disabled: props.disabled,
            on_change: move |open| {
                if let Some(callback) = &props.on_change {
                    callback.call(open);
                }
            },
            on_trigger_click: move |_| {
                if let Some(callback) = &props.on_trigger_click {
                    callback.call(());
                }
            },
            class: crate::cn::merge_slice(&["fts-accordion-item transition-colors", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// AccordionTrigger
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct AccordionTriggerProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Clickable header that toggles its parent `AccordionItem`.
#[component]
pub fn AccordionTrigger(props: AccordionTriggerProps) -> Element {
    rsx! {
        PrimitiveAccordionTrigger {
            id: props.id,
            class: crate::cn::merge(format!(
                "group flex w-full items-center justify-between gap-6 p-4 text-left text-sm font-medium text-foreground hover:underline cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring {}",
                props.class
            )),
            {props.children}
            svg {
                class: "size-4 text-muted-foreground transition-transform group-aria-expanded:rotate-180",
                xmlns: "http://www.w3.org/2000/svg",
                width: "24",
                height: "24",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                path { d: "m6 9 6 6 6-6" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// AccordionContent
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct AccordionContentProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Collapsible body of an `AccordionItem`. Only renders when its item is open.
#[component]
pub fn AccordionContent(props: AccordionContentProps) -> Element {
    rsx! {
        PrimitiveAccordionContent {
            id: props.id,
            class: crate::cn::merge_slice(&["px-4 text-sm text-muted-foreground", props.class.as_str()]),
            div {
                class: "pt-0 pb-4",
                {props.children}
            }
        }
    }
}

/// Default Accordion story with three FAQ-style items.
#[story(category = "Accordion", name = "default")]
pub fn accordion_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground max-w-xl",
            Accordion {
                AccordionItem { index: 0,
                    AccordionTrigger { "Is it accessible?" }
                    AccordionContent { "Yes — it follows WAI-ARIA accordion patterns." }
                }
                AccordionItem { index: 1,
                    AccordionTrigger { "Is it styled?" }
                    AccordionContent { "Yes — shadcn v4 maia tokens drive the look." }
                }
                AccordionItem { index: 2,
                    AccordionTrigger { "Is it animated?" }
                    AccordionContent { "Yes — open/close uses CSS transitions." }
                }
            }
        }
    }
}
