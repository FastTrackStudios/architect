//! Sheet (Side Sheet) — standalone, shadcn v4 maia style.

use dioxus::prelude::*;
use architect_story_runtime::story;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Which edge the sheet slides in from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SheetSide {
    Left,
    #[default]
    Right,
}

// ---------------------------------------------------------------------------
// Sheet (root)
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SheetProps {
    pub open: bool,
    #[props(default)]
    pub on_close: Option<Callback<()>>,
    #[props(default)]
    pub side: SheetSide,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: Sheet overlay + content panel.
#[component]
pub fn Sheet(props: SheetProps) -> Element {
    if !props.open {
        return rsx! {};
    }

    let on_close = props.on_close;

    let close_on_escape = move |e: KeyboardEvent| {
        if e.key() == Key::Escape {
            if let Some(cb) = &on_close {
                cb.call(());
            }
        }
    };

    let position_class = match props.side {
        SheetSide::Right => "fixed inset-y-0 right-0 z-50 w-3/4 sm:max-w-sm",
        SheetSide::Left => "fixed inset-y-0 left-0 z-50 w-3/4 sm:max-w-sm",
    };

    let border_class = match props.side {
        SheetSide::Right => "border-l border-border",
        SheetSide::Left => "border-r border-border",
    };

    rsx! {
        // Overlay
        div {
            class: "fixed inset-0 z-50 bg-black/80 supports-[backdrop-filter]:backdrop-blur-xs",
            onclick: move |_| {
                if let Some(cb) = &on_close {
                    cb.call(());
                }
            },
        }

        // Content
        div {
            class: crate::cn::merge(format!(
                "{position_class} flex flex-col h-full bg-popover text-popover-foreground {border_class} p-6 gap-4 outline-none {}",
                props.class
            )),
            tabindex: "-1",
            autofocus: true,
            onclick: move |evt: MouseEvent| { evt.stop_propagation(); },
            onkeydown: close_on_escape,

            // Close button
            SheetClose { on_close: on_close }

            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// SheetClose (X button)
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
struct SheetCloseProps {
    on_close: Option<Callback<()>>,
}

#[component]
fn SheetClose(props: SheetCloseProps) -> Element {
    let on_close = props.on_close;

    rsx! {
        button {
            class: "absolute top-4 right-4 opacity-70 hover:opacity-100 cursor-pointer",
            onclick: move |_| {
                if let Some(cb) = &on_close {
                    cb.call(());
                }
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
                line { x1: "18", y1: "6", x2: "6", y2: "18" }
                line { x1: "6", y1: "6", x2: "18", y2: "18" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct SheetHeaderProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-sheet-header
#[component]
pub fn SheetHeader(props: SheetHeaderProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-2", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SheetTitleProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-sheet-title
#[component]
pub fn SheetTitle(props: SheetTitleProps) -> Element {
    rsx! {
        h2 {
            class: crate::cn::merge_slice(&["text-base font-medium", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SheetDescriptionProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-sheet-description
#[component]
pub fn SheetDescription(props: SheetDescriptionProps) -> Element {
    rsx! {
        p {
            class: crate::cn::merge_slice(&["text-muted-foreground text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct SheetFooterProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-sheet-footer
#[component]
pub fn SheetFooter(props: SheetFooterProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col-reverse gap-2 sm:flex-row sm:justify-end mt-auto", props.class.as_str()]),
            {props.children}
        }
    }
}

/// Sheet side variants — Left / Right (the two values exposed by `SheetSide`).
/// Rendered as in-flow previews to keep the comparison in a single snapshot.
#[story(category = "Sheet", name = "sides")]
pub fn side_sheet_sides() -> Element {
    let preview = |label: &'static str, pos_class: &'static str| {
        rsx! {
            div { class: "relative h-56 w-72 rounded-md border border-border bg-muted/40 overflow-hidden",
                div {
                    class: "{pos_class} bg-popover text-popover-foreground border-border shadow-lg p-3 text-xs flex flex-col gap-1",
                    div { class: "font-medium", "Edit profile" }
                    div { class: "text-muted-foreground", "{label}" }
                }
            }
        }
    };
    rsx! {
        div { class: "p-6 bg-background text-foreground flex flex-wrap gap-6",
            div { class: "flex flex-col gap-2",
                span { class: "text-xs uppercase tracking-wider text-muted-foreground", "Left" }
                {preview("Slides from left", "absolute inset-y-0 left-0 w-2/3 border-r")}
            }
            div { class: "flex flex-col gap-2",
                span { class: "text-xs uppercase tracking-wider text-muted-foreground", "Right" }
                {preview("Slides from right", "absolute inset-y-0 right-0 w-2/3 border-l")}
            }
        }
    }
}

/// Side sheet forced open from the right edge.
#[story(category = "Sheet", name = "side sheet default")]
pub fn side_sheet_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground relative min-h-[24rem]",
            if !open() {
                button {
                    class: "h-9 px-3 rounded-lg border border-border text-sm",
                    onclick: move |_| open.set(true),
                    "Open sheet"
                }
            }
            Sheet {
                open: open(),
                on_close: move |_| open.set(false),
                side: SheetSide::Right,
                SheetHeader {
                    SheetTitle { "Edit profile" }
                    SheetDescription { "Make changes to your profile here." }
                }
                div { class: "text-sm text-muted-foreground", "Form fields would go here." }
                SheetFooter {
                    button {
                        class: "h-9 px-3 rounded-lg bg-primary text-primary-foreground text-sm",
                        onclick: move |_| open.set(false),
                        "Save"
                    }
                }
            }
        }
    }
}
