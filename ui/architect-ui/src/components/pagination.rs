//! Pagination — shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct PaginationProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Nav wrapper for pagination controls.
#[component]
pub fn Pagination(props: PaginationProps) -> Element {
    rsx! {
        nav {
            class: crate::cn::merge_slice(&["flex justify-center", props.class.as_str()]),
            role: "navigation",
            aria_label: "pagination",
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PaginationContentProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// The list container for pagination items.
#[component]
pub fn PaginationContent(props: PaginationContentProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex items-center gap-1", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PaginationItemProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Wraps each pagination button.
#[component]
pub fn PaginationItem(props: PaginationItemProps) -> Element {
    rsx! {
        div {
            class: props.class,
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PaginationLinkProps {
    pub page: u32,
    #[props(default = false)]
    pub is_active: bool,
    pub on_click: Callback<u32>,
    #[props(default)]
    pub class: String,
}

/// A page number button.
#[component]
pub fn PaginationLink(props: PaginationLinkProps) -> Element {
    let base = "inline-flex items-center justify-center h-9 min-w-9 rounded-lg border text-sm font-medium transition-colors";

    let state_class = if props.is_active {
        "border-input bg-input/30 text-foreground"
    } else {
        "border-transparent hover:bg-muted text-muted-foreground hover:text-foreground"
    };

    let page = props.page;

    rsx! {
        button {
            r#type: "button",
            class: crate::cn::merge_slice(&[base, state_class, props.class.as_str()]),
            aria_current: if props.is_active { Some("page") } else { None },
            onclick: move |_| {
                props.on_click.call(page);
            },
            "{page}"
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PaginationPreviousProps {
    pub on_click: Callback<()>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
}

/// Previous page button with left chevron.
#[component]
pub fn PaginationPrevious(props: PaginationPreviousProps) -> Element {
    let base = "inline-flex items-center justify-center h-9 min-w-9 rounded-lg border border-transparent text-sm font-medium transition-colors gap-1 pl-2";

    let disabled_class = if props.disabled {
        "pointer-events-none opacity-50"
    } else {
        "hover:bg-muted text-muted-foreground hover:text-foreground"
    };

    rsx! {
        button {
            r#type: "button",
            class: crate::cn::merge_slice(&[base, disabled_class, props.class.as_str()]),
            disabled: if props.disabled { Some(true) } else { None },
            onclick: move |_| {
                props.on_click.call(());
            },
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                width: "16",
                height: "16",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                path { d: "m15 18-6-6 6-6" }
            }
            "Previous"
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct PaginationNextProps {
    pub on_click: Callback<()>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
}

/// Next page button with right chevron.
#[component]
pub fn PaginationNext(props: PaginationNextProps) -> Element {
    let base = "inline-flex items-center justify-center h-9 min-w-9 rounded-lg border border-transparent text-sm font-medium transition-colors gap-1 pr-2";

    let disabled_class = if props.disabled {
        "pointer-events-none opacity-50"
    } else {
        "hover:bg-muted text-muted-foreground hover:text-foreground"
    };

    rsx! {
        button {
            r#type: "button",
            class: crate::cn::merge_slice(&[base, disabled_class, props.class.as_str()]),
            disabled: if props.disabled { Some(true) } else { None },
            onclick: move |_| {
                props.on_click.call(());
            },
            "Next"
            svg {
                xmlns: "http://www.w3.org/2000/svg",
                width: "16",
                height: "16",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                path { d: "m9 18 6-6-6-6" }
            }
        }
    }
}

/// Ellipsis indicator for skipped pages.
#[component]
pub fn PaginationEllipsis() -> Element {
    rsx! {
        span {
            class: "flex size-9 items-center justify-center text-muted-foreground",
            "..."
        }
    }
}

/// Many-page layout — `1 ... 5 6 7 ... 20` with the middle page active.
#[story(category = "Pagination", name = "many-pages")]
pub fn pagination_many_pages() -> Element {
    let mut current = use_signal(|| 6u32);
    let total: u32 = 20;
    let on_select = use_callback(move |p: u32| current.set(p));
    let on_prev = use_callback(move |_| {
        let c = current();
        if c > 1 {
            current.set(c - 1);
        }
    });
    let on_next = use_callback(move |_| {
        let c = current();
        if c < total {
            current.set(c + 1);
        }
    });
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Pagination {
                PaginationContent {
                    PaginationItem {
                        PaginationPrevious { on_click: on_prev, disabled: current() == 1 }
                    }
                    PaginationItem {
                        PaginationLink { page: 1u32, is_active: current() == 1, on_click: on_select }
                    }
                    PaginationItem { PaginationEllipsis {} }
                    PaginationItem {
                        PaginationLink { page: 5u32, is_active: current() == 5, on_click: on_select }
                    }
                    PaginationItem {
                        PaginationLink { page: 6u32, is_active: current() == 6, on_click: on_select }
                    }
                    PaginationItem {
                        PaginationLink { page: 7u32, is_active: current() == 7, on_click: on_select }
                    }
                    PaginationItem { PaginationEllipsis {} }
                    PaginationItem {
                        PaginationLink { page: 20u32, is_active: current() == 20, on_click: on_select }
                    }
                    PaginationItem {
                        PaginationNext { on_click: on_next, disabled: current() == total }
                    }
                }
            }
        }
    }
}

/// Pagination with previous/next, numeric pages, ellipsis, and an active page.
#[story(category = "Pagination", name = "default")]
pub fn pagination_default() -> Element {
    let mut current = use_signal(|| 2u32);
    let total: u32 = 10;
    let on_select = use_callback(move |p: u32| current.set(p));
    let on_prev = use_callback(move |_| {
        let c = current();
        if c > 1 {
            current.set(c - 1);
        }
    });
    let on_next = use_callback(move |_| {
        let c = current();
        if c < total {
            current.set(c + 1);
        }
    });

    rsx! {
        div { class: "p-6 bg-background text-foreground",
            Pagination {
                PaginationContent {
                    PaginationItem {
                        PaginationPrevious { on_click: on_prev, disabled: current() == 1 }
                    }
                    PaginationItem {
                        PaginationLink { page: 1u32, is_active: current() == 1, on_click: on_select }
                    }
                    PaginationItem {
                        PaginationLink { page: 2u32, is_active: current() == 2, on_click: on_select }
                    }
                    PaginationItem {
                        PaginationLink { page: 3u32, is_active: current() == 3, on_click: on_select }
                    }
                    PaginationItem { PaginationEllipsis {} }
                    PaginationItem {
                        PaginationLink { page: 10u32, is_active: current() == 10, on_click: on_select }
                    }
                    PaginationItem {
                        PaginationNext { on_click: on_next, disabled: current() == total }
                    }
                }
            }
        }
    }
}
