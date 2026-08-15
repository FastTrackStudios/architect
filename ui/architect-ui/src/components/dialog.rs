//! Dialog — shadcn v4 maia style modal overlay.

use dioxus::prelude::*;
use dioxus_primitives::dialog::{
    DialogContent as PrimitiveDialogContent, DialogDescription as PrimitiveDialogDescription,
    DialogRoot as PrimitiveDialog, DialogTitle as PrimitiveDialogTitle,
};
use architect_story_runtime::story;

#[derive(Props, Clone, PartialEq)]
pub struct DialogProps {
    #[props(default)]
    pub open: Option<bool>,
    #[props(default = false)]
    pub default_open: bool,
    #[props(default = true)]
    pub is_modal: bool,
    #[props(default)]
    pub on_open_change: Option<Callback<bool>>,
    #[props(default)]
    pub on_close: Option<Callback<()>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-dialog-overlay + cn-dialog-content
#[component]
pub fn Dialog(props: DialogProps) -> Element {
    rsx! {
        PrimitiveDialog {
            open: props.open,
            default_open: props.default_open,
            is_modal: props.is_modal,
            on_open_change: move |open| {
                if let Some(callback) = &props.on_open_change {
                    callback.call(open);
                }
                if !open {
                    if let Some(callback) = &props.on_close {
                        callback.call(());
                    }
                }
            },
            class: "fixed inset-0 z-50 bg-black/80 animate-fade-in supports-[backdrop-filter]:backdrop-blur-xs",
            PrimitiveDialogContent {
                class: crate::cn::merge(format!(
                    "fixed z-50 grid w-full max-w-[calc(100%-2rem)] sm:max-w-md gap-6 rounded-xl bg-popover text-popover-foreground border border-border shadow-lg p-6 text-sm animate-scale-in {}",
                    props.class
                )),
                style: "left: 50%; top: 50%; transform: translate(-50%, -50%);",
                {props.children}
            }
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DialogHeaderProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-dialog-header
#[component]
pub fn DialogHeader(props: DialogHeaderProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-2", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DialogTitleProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-dialog-title
#[component]
pub fn DialogTitle(props: DialogTitleProps) -> Element {
    rsx! {
        PrimitiveDialogTitle {
            id: props.id,
            class: crate::cn::merge_slice(&["text-base leading-none font-medium", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DialogDescriptionProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-dialog-description
#[component]
pub fn DialogDescription(props: DialogDescriptionProps) -> Element {
    rsx! {
        PrimitiveDialogDescription {
            id: props.id,
            class: crate::cn::merge_slice(&["text-sm text-muted-foreground", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DialogFooterProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-dialog-footer
#[component]
pub fn DialogFooter(props: DialogFooterProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col-reverse gap-2 sm:flex-row sm:justify-end", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct DialogCloseProps {
    pub on_click: Callback<()>,
    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: cn-dialog-close
#[component]
pub fn DialogClose(props: DialogCloseProps) -> Element {
    rsx! {
        button {
            class: crate::cn::merge(format!(
                "absolute top-4 right-4 rounded-sm opacity-70 transition-opacity hover:opacity-100 focus:outline-none focus:ring-2 focus:ring-ring {}",
                props.class
            )),
            r#type: "button",
            onclick: move |_| props.on_click.call(()),
            span { class: "size-4 text-lg leading-none", aria_hidden: "true", "\u{2715}" }
            span { class: "sr-only", "Close" }
        }
    }
}

/// Dialog forced open with header, description, and footer actions.
#[story(category = "Dialog", name = "dialog default")]
pub fn dialog_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground relative min-h-[24rem]",
            if !open() {
                button {
                    class: "h-9 px-3 rounded-lg border border-border text-sm",
                    onclick: move |_| open.set(true),
                    "Open dialog"
                }
            }
            Dialog {
                open: open(),
                on_close: move |_| open.set(false),
                DialogHeader {
                    DialogTitle { "Confirm action" }
                    DialogDescription { "This is a forced-open dialog used for snapshot testing." }
                }
                p { class: "text-sm text-muted-foreground", "Body content goes here." }
                DialogFooter {
                    button {
                        class: "h-9 px-3 rounded-lg border border-border text-sm",
                        onclick: move |_| open.set(false),
                        "Cancel"
                    }
                    button {
                        class: "h-9 px-3 rounded-lg bg-primary text-primary-foreground text-sm",
                        onclick: move |_| open.set(false),
                        "Confirm"
                    }
                }
            }
        }
    }
}
