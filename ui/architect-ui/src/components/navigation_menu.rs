//! NavigationMenu — shadcn v4 maia style top-level navigation with mega-menu dropdowns.

use architect_story_runtime::story;
use dioxus::prelude::*;

// ── Context ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct NavigationMenuContext {
    active: Signal<Option<String>>,
}

#[derive(Clone)]
struct NavigationMenuItemContext {
    value: String,
}

// ── NavigationMenu ─────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct NavigationMenuProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: navigation-menu root
#[component]
pub fn NavigationMenu(props: NavigationMenuProps) -> Element {
    let active = use_signal(|| None::<String>);
    use_context_provider(|| NavigationMenuContext { active });

    rsx! {
        nav {
            class: crate::cn::merge_slice(&["relative flex max-w-max items-center", props.class.as_str()]),
            {props.children}
        }
    }
}

// ── NavigationMenuList ─────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct NavigationMenuListProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: navigation-menu-list
#[component]
pub fn NavigationMenuList(props: NavigationMenuListProps) -> Element {
    rsx! {
        ul {
            class: crate::cn::merge_slice(&["flex items-center gap-0", props.class.as_str()]),
            {props.children}
        }
    }
}

// ── NavigationMenuItem ─────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct NavigationMenuItemProps {
    /// Unique identifier for this menu item.
    pub value: String,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: navigation-menu-item
#[component]
pub fn NavigationMenuItem(props: NavigationMenuItemProps) -> Element {
    use_context_provider(|| NavigationMenuItemContext {
        value: props.value.clone(),
    });

    rsx! {
        li {
            class: crate::cn::merge_slice(&["relative", props.class.as_str()]),
            {props.children}
        }
    }
}

// ── NavigationMenuTrigger ──────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct NavigationMenuTriggerProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: navigation-menu-trigger
#[component]
pub fn NavigationMenuTrigger(props: NavigationMenuTriggerProps) -> Element {
    let mut ctx: NavigationMenuContext = use_context();
    let item_ctx: NavigationMenuItemContext = use_context();
    let item_value = item_ctx.value.clone();

    let is_open = ctx
        .active
        .read()
        .as_ref()
        .map(|v| v == &item_value)
        .unwrap_or(false);

    let open_class = if is_open { "bg-muted/50" } else { "" };
    let chevron_rotate = if is_open { "rotate-180" } else { "" };

    let toggle_value = item_value.clone();

    rsx! {
        button {
            r#type: "button",
            class: crate::cn::merge(format!(
                "group/navigation-menu-trigger inline-flex items-center justify-center rounded-lg px-4 py-2.5 text-sm font-medium transition-all hover:bg-muted focus:bg-muted cursor-pointer select-none gap-1 {open_class} {}",
                props.class
            )),
            onclick: move |_| {
                let current = ctx.active.read().clone();
                if current.as_deref() == Some(&toggle_value) {
                    ctx.active.set(None);
                } else {
                    ctx.active.set(Some(toggle_value.clone()));
                }
            },
            {props.children}
            // Chevron icon
            svg {
                class: "relative top-px ml-1 size-3 transition duration-300 {chevron_rotate}",
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

// ── NavigationMenuContent ──────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct NavigationMenuContentProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: navigation-menu-content dropdown
#[component]
pub fn NavigationMenuContent(props: NavigationMenuContentProps) -> Element {
    let mut ctx: NavigationMenuContext = use_context();
    let item_ctx: NavigationMenuItemContext = use_context();

    let is_active = ctx
        .active
        .read()
        .as_ref()
        .map(|v| v == &item_ctx.value)
        .unwrap_or(false);

    if !is_active {
        return rsx! {};
    }

    rsx! {
        // Invisible backdrop for click-outside dismissal
        div {
            class: "fixed inset-0 z-[99]",
            onclick: move |_| ctx.active.set(None),
        }
        // Content panel
        div {
            class: crate::cn::merge(format!(
                "absolute left-0 top-full z-[100] mt-1 rounded-lg bg-popover text-popover-foreground border border-border shadow-md p-2.5 pr-3 {}",
                props.class
            )),
            onclick: move |e| e.stop_propagation(),
            {props.children}
        }
    }
}

// ── NavigationMenuLink ─────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct NavigationMenuLinkProps {
    #[props(default)]
    pub active: bool,
    #[props(default)]
    pub href: Option<String>,
    #[props(default)]
    pub on_click: Option<Callback<()>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: navigation-menu-link
#[component]
pub fn NavigationMenuLink(props: NavigationMenuLinkProps) -> Element {
    let mut ctx: NavigationMenuContext = use_context();

    let active_class = if props.active { "bg-muted/50" } else { "" };
    let base = "flex items-center gap-1.5 rounded-xl p-3 text-sm transition-all hover:bg-muted focus:bg-muted outline-none [&_svg:not([class*='size-'])]:size-4";

    let href = props.href.clone().unwrap_or_default();
    let has_href = props.href.is_some();

    rsx! {
        a {
            class: crate::cn::merge_slice(&[base, active_class, props.class.as_str()]),
            href: if has_href { href } else { String::new() },
            onclick: move |_| {
                if let Some(cb) = &props.on_click {
                    cb.call(());
                }
                ctx.active.set(None);
            },
            {props.children}
        }
    }
}

/// NavigationMenu with two items, one with a dropdown content panel.
#[story(category = "NavigationMenu", name = "default")]
pub fn navigation_menu_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground min-h-64",
            NavigationMenu {
                NavigationMenuList {
                    NavigationMenuItem { value: "products".to_string(),
                        NavigationMenuTrigger { "Products" }
                        NavigationMenuContent {
                            div { class: "grid gap-2 w-64",
                                NavigationMenuLink { href: "#analytics".to_string(), "Analytics" }
                                NavigationMenuLink { href: "#engagement".to_string(), active: true, "Engagement" }
                                NavigationMenuLink { href: "#security".to_string(), "Security" }
                            }
                        }
                    }
                    NavigationMenuItem { value: "company".to_string(),
                        NavigationMenuTrigger { "Company" }
                        NavigationMenuContent {
                            div { class: "grid gap-2 w-64",
                                NavigationMenuLink { href: "#about".to_string(), "About" }
                                NavigationMenuLink { href: "#careers".to_string(), "Careers" }
                            }
                        }
                    }
                }
            }
        }
    }
}
