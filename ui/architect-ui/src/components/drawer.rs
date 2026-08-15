//! Drawer — shadcn v4 maia style bottom/side drawer (mobile-friendly sheet variant).

use architect_story_runtime::story;
use dioxus::prelude::*;

/// Which edge the drawer slides in from.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum DrawerSide {
    #[default]
    Bottom,
    Left,
    Right,
}

#[derive(Props, Clone, PartialEq)]
pub struct DrawerProps {
    pub open: bool,
    #[props(default)]
    pub on_close: Option<Callback<()>>,
    #[props(default)]
    pub side: DrawerSide,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-drawer-overlay + cn-drawer-content
#[component]
pub fn Drawer(props: DrawerProps) -> Element {
    if !props.open {
        return rsx! {};
    }

    let position_classes = match props.side {
        DrawerSide::Bottom => "fixed inset-x-0 bottom-0 z-50 mt-24 max-h-[80vh] rounded-t-xl",
        DrawerSide::Left => "fixed inset-y-0 left-0 z-50 w-3/4 sm:max-w-sm rounded-r-xl",
        DrawerSide::Right => "fixed inset-y-0 right-0 z-50 w-3/4 sm:max-w-sm rounded-l-xl",
    };

    let on_close = props.on_close;
    let close_on_escape = move |e: KeyboardEvent| {
        if e.key() == Key::Escape {
            if let Some(cb) = &on_close {
                cb.call(());
            }
        }
    };

    rsx! {
        // Overlay — closes on click
        div {
            class: "fixed inset-0 z-50 bg-black/80 animate-fade-in supports-[backdrop-filter]:backdrop-blur-xs",
            "data-state": "open",
            onclick: move |_| {
                if let Some(cb) = &on_close {
                    cb.call(());
                }
            },
        }

        // Content
        div {
            class: crate::cn::merge(format!(
                "{position_classes} flex flex-col bg-popover text-popover-foreground border border-border shadow-lg p-4 text-sm outline-none {}",
                props.class
            )),
            role: "dialog",
            aria_modal: "true",
            tabindex: "-1",
            autofocus: true,
            onclick: move |evt: MouseEvent| {
                evt.stop_propagation();
            },
            onkeydown: close_on_escape,
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DrawerHeaderProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-drawer-header
#[component]
pub fn DrawerHeader(props: DrawerHeaderProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-0.5 p-4", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DrawerTitleProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-drawer-title
#[component]
pub fn DrawerTitle(props: DrawerTitleProps) -> Element {
    rsx! {
        h2 {
            class: crate::cn::merge_slice(&["text-foreground text-base font-medium", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DrawerDescriptionProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-drawer-description
#[component]
pub fn DrawerDescription(props: DrawerDescriptionProps) -> Element {
    rsx! {
        p {
            class: crate::cn::merge_slice(&["text-muted-foreground text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DrawerFooterProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-drawer-footer
#[component]
pub fn DrawerFooter(props: DrawerFooterProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-2 p-4", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DrawerHandleProps {
    #[props(default)]
    pub class: String,
}

/// Drag handle indicator for bottom drawers.
#[component]
pub fn DrawerHandle(props: DrawerHandleProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge(format!(
                "mx-auto mt-4 h-1.5 w-[100px] shrink-0 rounded-full bg-muted {}",
                props.class
            )),
        }
    }
}

/// Drawer side variants — Left / Right / Bottom (no Top in `DrawerSide`).
/// Renders three small previews side-by-side rather than full-screen overlays
/// so the matrix is comparable in one snapshot.
#[story(category = "Drawer", name = "sides")]
pub fn drawer_sides() -> Element {
    let preview = |label: &'static str, pos_class: &'static str| {
        rsx! {
            div { class: "relative h-48 w-64 rounded-md border border-border bg-muted/40 overflow-hidden",
                div {
                    class: "{pos_class} bg-popover text-popover-foreground border border-border shadow-lg p-3 text-xs",
                    div { class: "font-medium mb-1", "Drawer" }
                    div { class: "text-muted-foreground", "{label}" }
                }
            }
        }
    };
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-wrap gap-6",
            div { class: "flex flex-col gap-2",
                span { class: "text-xs uppercase tracking-wider text-muted-foreground", "Left" }
                {preview("Slides from left", "absolute inset-y-0 left-0 w-2/3 rounded-r-xl")}
            }
            div { class: "flex flex-col gap-2",
                span { class: "text-xs uppercase tracking-wider text-muted-foreground", "Right" }
                {preview("Slides from right", "absolute inset-y-0 right-0 w-2/3 rounded-l-xl")}
            }
            div { class: "flex flex-col gap-2",
                span { class: "text-xs uppercase tracking-wider text-muted-foreground", "Bottom" }
                {preview("Slides from bottom", "absolute inset-x-0 bottom-0 h-2/3 rounded-t-xl")}
            }
        }
    }
}

/// Drawer forced open from the right with header, body, and footer.
#[story(category = "Drawer", name = "drawer default")]
pub fn drawer_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground relative min-h-[24rem]",
            if !open() {
                button {
                    class: "h-9 px-3 rounded-lg border border-border text-sm",
                    onclick: move |_| open.set(true),
                    "Open drawer"
                }
            }
            Drawer {
                open: open(),
                on_close: move |_| open.set(false),
                side: DrawerSide::Right,
                DrawerHeader {
                    DrawerTitle { "Drawer" }
                    DrawerDescription { "A right-side drawer rendered open for snapshots." }
                }
                div { class: "px-4 text-sm text-muted-foreground", "Drawer content body." }
                DrawerFooter {
                    button {
                        class: "h-9 px-3 rounded-lg border border-border text-sm",
                        onclick: move |_| open.set(false),
                        "Close"
                    }
                }
            }
        }
    }
}
