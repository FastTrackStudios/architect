//! Breadcrumb — shadcn v4 maia style breadcrumb navigation.

use architect_story_runtime::story;
use dioxus::prelude::*;

#[derive(Props, Clone, PartialEq)]
pub struct BreadcrumbProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-breadcrumb
#[component]
pub fn Breadcrumb(props: BreadcrumbProps) -> Element {
    rsx! {
        nav {
            class: props.class,
            aria_label: "breadcrumb",
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct BreadcrumbListProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-breadcrumb-list
#[component]
pub fn BreadcrumbList(props: BreadcrumbListProps) -> Element {
    rsx! {
        ol {
            class: crate::cn::merge(format!(
                "flex items-center flex-wrap text-muted-foreground gap-1.5 text-sm sm:gap-2.5 {}",
                props.class
            )),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct BreadcrumbItemProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-breadcrumb-item
#[component]
pub fn BreadcrumbItem(props: BreadcrumbItemProps) -> Element {
    rsx! {
        li {
            class: crate::cn::merge_slice(&["inline-flex items-center gap-1.5", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct BreadcrumbLinkProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-breadcrumb-link
#[component]
pub fn BreadcrumbLink(props: BreadcrumbLinkProps) -> Element {
    rsx! {
        a {
            class: crate::cn::merge_slice(&["hover:text-foreground transition-colors", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct BreadcrumbPageProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-breadcrumb-page — current page, not a link.
#[component]
pub fn BreadcrumbPage(props: BreadcrumbPageProps) -> Element {
    rsx! {
        span {
            class: crate::cn::merge_slice(&["text-foreground font-normal", props.class.as_str()]),
            role: "link",
            aria_disabled: "true",
            aria_current: "page",
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct BreadcrumbSeparatorProps {
    #[props(default)]
    pub class: String,
    /// Optional custom separator element. Defaults to a chevron-right SVG.
    #[props(default)]
    pub children: Element,
}

/// shadcn v4 maia: cn-breadcrumb-separator — chevron-right icon by default.
#[component]
pub fn BreadcrumbSeparator(props: BreadcrumbSeparatorProps) -> Element {
    rsx! {
        li {
            class: crate::cn::merge_slice(&["[&>svg]:size-3.5", props.class.as_str()]),
            role: "presentation",
            aria_hidden: "true",
            if props.children == VNode::empty() {
                // Default chevron-right icon
                svg {
                    class: "size-3.5",
                    xmlns: "http://www.w3.org/2000/svg",
                    width: "24",
                    height: "24",
                    view_box: "0 0 24 24",
                    fill: "none",
                    stroke: "currentColor",
                    stroke_width: "2",
                    stroke_linecap: "round",
                    stroke_linejoin: "round",
                    path { d: "m9 18 6-6-6-6" }
                }
            } else {
                {props.children}
            }
        }
    }
}

/// Long breadcrumb path collapsed with a `...` ellipsis between root and tail.
#[story(category = "Breadcrumb", name = "with-ellipsis")]
pub fn breadcrumb_with_ellipsis() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Breadcrumb {
                BreadcrumbList {
                    BreadcrumbItem { BreadcrumbLink { "Home" } }
                    BreadcrumbSeparator {}
                    BreadcrumbItem {
                        span { class: "text-muted-foreground", "..." }
                    }
                    BreadcrumbSeparator {}
                    BreadcrumbItem { BreadcrumbLink { "Components" } }
                    BreadcrumbSeparator {}
                    BreadcrumbItem { BreadcrumbLink { "Navigation" } }
                    BreadcrumbSeparator {}
                    BreadcrumbItem { BreadcrumbPage { "Breadcrumb" } }
                }
            }
        }
    }
}

/// Default breadcrumb trail with separators and a current page.
#[story(category = "Breadcrumb", name = "default")]
pub fn breadcrumb_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Breadcrumb {
                BreadcrumbList {
                    BreadcrumbItem { BreadcrumbLink { "Home" } }
                    BreadcrumbSeparator {}
                    BreadcrumbItem { BreadcrumbLink { "Components" } }
                    BreadcrumbSeparator {}
                    BreadcrumbItem { BreadcrumbPage { "Breadcrumb" } }
                }
            }
        }
    }
}
