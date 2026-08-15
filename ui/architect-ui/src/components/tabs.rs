//! Tabs — shadcn v4 maia style accessible tabbed interface.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::tabs::{
    TabContent as PrimitiveTabContent, TabList as PrimitiveTabList,
    TabTrigger as PrimitiveTabTrigger, Tabs as PrimitiveTabs,
};

#[derive(Props, Clone, PartialEq)]
pub struct TabsProps {
    #[props(default)]
    pub value: Option<String>,
    #[props(default = String::new())]
    pub default_value: String,
    #[props(default)]
    pub on_change: Option<Callback<String>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default = true)]
    pub horizontal: bool,
    #[props(default = true)]
    pub roving_loop: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-tabs
#[component]
pub fn Tabs(props: TabsProps) -> Element {
    rsx! {
        document::Style {
            r#"
                .fts-tab-trigger[data-state="active"] {{
                    background: var(--background);
                    color: var(--foreground);
                    box-shadow: var(--shadow-sm, 0 1px 2px 0 rgb(0 0 0 / 0.05));
                }}

                .fts-tab-trigger[data-state="inactive"]:hover {{
                    background: color-mix(in oklab, var(--background) 50%, transparent);
                    color: color-mix(in oklab, var(--foreground) 80%, transparent);
                }}
            "#
        }
        PrimitiveTabs {
            value: props.value,
            default_value: props.default_value,
            disabled: props.disabled,
            horizontal: props.horizontal,
            roving_loop: props.roving_loop,
            on_value_change: move |value: String| {
                if let Some(callback) = &props.on_change {
                    callback.call(value);
                }
            },
            class: crate::cn::merge_slice(&["flex flex-col gap-2", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TabListProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-tabs-list
#[component]
pub fn TabList(props: TabListProps) -> Element {
    rsx! {
        PrimitiveTabList {
            class: crate::cn::merge(format!(
                "inline-flex h-9 items-center justify-start gap-1 rounded-full bg-muted p-[3px] text-muted-foreground {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TabTriggerProps {
    pub value: String,
    pub index: usize,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-tabs-trigger
#[component]
pub fn TabTrigger(props: TabTriggerProps) -> Element {
    rsx! {
        PrimitiveTabTrigger {
            value: props.value,
            index: props.index,
            disabled: props.disabled,
            id: props.id,
            class: crate::cn::merge(format!(
                "fts-tab-trigger inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-xl border border-transparent px-2 py-1 text-sm font-medium transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring disabled:pointer-events-none disabled:opacity-50 [&_svg:not([class*='size-'])]:size-4 {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct TabContentProps {
    pub value: String,
    pub index: usize,
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-tabs-content
#[component]
pub fn TabContent(props: TabContentProps) -> Element {
    rsx! {
        PrimitiveTabContent {
            value: props.value,
            index: props.index,
            id: props.id,
            class: crate::cn::merge_slice(&["text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

/// Tabs in vertical orientation — `horizontal: false`. The TabList is
/// flipped to a column and placed beside the content panels.
#[story(category = "Tabs", name = "vertical")]
pub fn tabs_vertical() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Tabs {
                default_value: "account".to_string(),
                horizontal: false,
                class: "flex flex-row gap-4".to_string(),
                TabList {
                    class: "flex-col h-auto items-stretch gap-1 rounded-xl p-1".to_string(),
                    TabTrigger { value: "account".to_string(), index: 0, "Account" }
                    TabTrigger { value: "password".to_string(), index: 1, "Password" }
                    TabTrigger { value: "settings".to_string(), index: 2, "Settings" }
                }
                div { class: "flex-1",
                    TabContent { value: "account".to_string(), index: 0,
                        div { class: "p-3 text-muted-foreground", "Account details go here." }
                    }
                    TabContent { value: "password".to_string(), index: 1,
                        div { class: "p-3 text-muted-foreground", "Password update form." }
                    }
                    TabContent { value: "settings".to_string(), index: 2,
                        div { class: "p-3 text-muted-foreground", "Settings panel." }
                    }
                }
            }
        }
    }
}

/// Three-tab layout with content panels.
#[story(category = "Tabs", name = "default")]
pub fn tabs_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Tabs { default_value: "account".to_string(),
                TabList {
                    TabTrigger { value: "account".to_string(), index: 0, "Account" }
                    TabTrigger { value: "password".to_string(), index: 1, "Password" }
                    TabTrigger { value: "settings".to_string(), index: 2, "Settings" }
                }
                TabContent { value: "account".to_string(), index: 0,
                    div { class: "p-3 text-muted-foreground", "Account details go here." }
                }
                TabContent { value: "password".to_string(), index: 1,
                    div { class: "p-3 text-muted-foreground", "Password update form." }
                }
                TabContent { value: "settings".to_string(), index: 2,
                    div { class: "p-3 text-muted-foreground", "Settings panel." }
                }
            }
        }
    }
}
