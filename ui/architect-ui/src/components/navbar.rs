//! Navbar — router-gated primitive-backed navigation menu.

use dioxus::prelude::*;
use dioxus_primitives::navbar::{
    Navbar as PrimitiveNavbar, NavbarContent as PrimitiveNavbarContent,
    NavbarItem as PrimitiveNavbarItem, NavbarNav as PrimitiveNavbarNav,
    NavbarTrigger as PrimitiveNavbarTrigger,
};
use architect_story_runtime::story;

#[component]
pub fn Navbar(
    #[props(default = false)] disabled: bool,
    #[props(default = true)] roving_loop: bool,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    rsx! {
        PrimitiveNavbar {
            disabled,
            roving_loop,
            class: crate::cn::merge_slice(&["relative flex items-center gap-1", class.as_str()]),
            {children}
        }
    }
}

#[component]
pub fn NavbarNav(
    index: usize,
    #[props(default = false)] disabled: bool,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    rsx! {
        PrimitiveNavbarNav {
            index,
            disabled,
            class: crate::cn::merge_slice(&["relative", class.as_str()]),
            {children}
        }
    }
}

#[component]
pub fn NavbarTrigger(#[props(default)] class: String, children: Element) -> Element {
    rsx! {
        PrimitiveNavbarTrigger {
            class: crate::cn::merge_slice(&[
                "inline-flex h-9 items-center justify-center rounded-lg px-3 text-sm font-medium transition-colors hover:bg-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                class.as_str(),
            ]),
            {children}
        }
    }
}

#[component]
pub fn NavbarContent(
    #[props(default)] id: Option<String>,
    #[props(default)] class: String,
    children: Element,
) -> Element {
    rsx! {
        PrimitiveNavbarContent {
            id,
            class: crate::cn::merge_slice(&[
                "absolute left-0 top-full z-50 mt-2 min-w-56 rounded-lg border border-border bg-popover p-2 text-popover-foreground shadow-md",
                class.as_str(),
            ]),
            {children}
        }
    }
}

#[component]
pub fn NavbarItem(
    index: usize,
    value: String,
    to: NavigationTarget,
    #[props(default = false)] disabled: bool,
    #[props(default)] on_select: Option<Callback<String>>,
    #[props(default)] class: Option<String>,
    #[props(default)] active_class: Option<String>,
    #[props(default = false)] new_tab: bool,
    #[props(default)] rel: Option<String>,
    #[props(default = false)] onclick_only: bool,
    children: Element,
) -> Element {
    let class = Some(crate::cn::merge_slice(&[
        "flex items-center rounded-md px-3 py-2 text-sm transition-colors hover:bg-accent hover:text-accent-foreground data-[disabled=true]:pointer-events-none data-[disabled=true]:opacity-50",
        class.as_deref().unwrap_or_default(),
    ]));

    rsx! {
        PrimitiveNavbarItem {
            index,
            value,
            to,
            disabled,
            class,
            active_class,
            new_tab,
            rel,
            onclick_only,
            on_select: move |value| {
                if let Some(callback) = &on_select {
                    callback.call(value);
                }
            },
            {children}
        }
    }
}

/// Navbar with a single nav containing a trigger and dropdown content.
#[story(category = "Navbar", name = "default")]
pub fn navbar_default() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground min-h-48",
            Navbar {
                NavbarNav { index: 0,
                    NavbarTrigger { "Products" }
                    NavbarContent {
                        div { class: "flex flex-col gap-1",
                            div { class: "px-3 py-2 text-sm rounded-md hover:bg-accent hover:text-accent-foreground", "Analytics" }
                            div { class: "px-3 py-2 text-sm rounded-md hover:bg-accent hover:text-accent-foreground", "Engagement" }
                            div { class: "px-3 py-2 text-sm rounded-md hover:bg-accent hover:text-accent-foreground", "Security" }
                        }
                    }
                }
                NavbarNav { index: 1,
                    NavbarTrigger { "Company" }
                    NavbarContent {
                        div { class: "flex flex-col gap-1",
                            div { class: "px-3 py-2 text-sm rounded-md hover:bg-accent hover:text-accent-foreground", "About" }
                            div { class: "px-3 py-2 text-sm rounded-md hover:bg-accent hover:text-accent-foreground", "Careers" }
                        }
                    }
                }
            }
        }
    }
}
