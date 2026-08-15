//! AlertDialog — primitive-backed confirmation dialog for destructive actions.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::alert_dialog::{
    AlertDialogAction as PrimitiveAlertDialogAction,
    AlertDialogCancel as PrimitiveAlertDialogCancel,
    AlertDialogContent as PrimitiveAlertDialogContent,
    AlertDialogDescription as PrimitiveAlertDialogDescription,
    AlertDialogRoot as PrimitiveAlertDialog, AlertDialogTitle as PrimitiveAlertDialogTitle,
};

#[derive(Props, Clone, PartialEq)]
pub struct AlertDialogProps {
    #[props(default)]
    pub id: Option<String>,
    #[props(default)]
    pub open: Option<bool>,
    #[props(default = false)]
    pub default_open: bool,
    #[props(default)]
    pub on_open_change: Option<Callback<bool>>,
    #[props(default)]
    pub on_close: Option<Callback<()>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn AlertDialog(props: AlertDialogProps) -> Element {
    rsx! {
        PrimitiveAlertDialog {
            id: props.id,
            open: props.open,
            default_open: props.default_open,
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
            PrimitiveAlertDialogContent {
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
pub struct AlertDialogHeaderProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn AlertDialogHeader(props: AlertDialogHeaderProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col gap-1.5 text-center sm:text-left", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AlertDialogTitleProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn AlertDialogTitle(props: AlertDialogTitleProps) -> Element {
    rsx! {
        PrimitiveAlertDialogTitle {
            class: crate::cn::merge_slice(&["text-lg font-medium", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AlertDialogDescriptionProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn AlertDialogDescription(props: AlertDialogDescriptionProps) -> Element {
    rsx! {
        PrimitiveAlertDialogDescription {
            class: crate::cn::merge_slice(&["text-muted-foreground text-sm", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AlertDialogFooterProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn AlertDialogFooter(props: AlertDialogFooterProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex flex-col-reverse gap-2 sm:flex-row sm:justify-end", props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AlertDialogActionProps {
    #[props(default)]
    pub on_click: Option<EventHandler<MouseEvent>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn AlertDialogAction(props: AlertDialogActionProps) -> Element {
    rsx! {
        PrimitiveAlertDialogAction {
            on_click: props.on_click,
            class: props.class,
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AlertDialogCancelProps {
    #[props(default)]
    pub on_click: Option<EventHandler<MouseEvent>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn AlertDialogCancel(props: AlertDialogCancelProps) -> Element {
    rsx! {
        PrimitiveAlertDialogCancel {
            on_click: props.on_click,
            class: props.class,
            {props.children}
        }
    }
}

/// AlertDialog with an emphasised destructive primary action.
#[story(category = "AlertDialog", name = "destructive")]
pub fn alert_dialog_destructive() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground relative min-h-[24rem]",
            if !open() {
                button {
                    class: "h-9 px-3 rounded-lg border border-border text-sm",
                    onclick: move |_| open.set(true),
                    "Open destructive dialog"
                }
            }
            AlertDialog {
                open: open(),
                on_close: move |_| open.set(false),
                AlertDialogHeader {
                    AlertDialogTitle { "Delete project?" }
                    AlertDialogDescription { "Deleting this project will permanently remove all of its data, including settings and history. This cannot be undone." }
                }
                AlertDialogFooter {
                    AlertDialogCancel {
                        class: "h-9 px-3 rounded-lg border border-border text-sm".to_string(),
                        on_click: move |_| open.set(false),
                        "Cancel"
                    }
                    AlertDialogAction {
                        class: "h-9 px-3 rounded-lg bg-destructive text-destructive-foreground hover:bg-destructive/90 font-medium text-sm".to_string(),
                        on_click: move |_| open.set(false),
                        "Delete project"
                    }
                }
            }
        }
    }
}

/// AlertDialog forced open with destructive confirmation actions.
#[story(category = "AlertDialog", name = "alert dialog default")]
pub fn alert_dialog_default() -> Element {
    let mut open = use_signal(|| true);
    rsx! {
        div { class: "p-6 bg-background text-foreground relative min-h-[24rem]",
            if !open() {
                button {
                    class: "h-9 px-3 rounded-lg border border-border text-sm",
                    onclick: move |_| open.set(true),
                    "Open alert dialog"
                }
            }
            AlertDialog {
                open: open(),
                on_close: move |_| open.set(false),
                AlertDialogHeader {
                    AlertDialogTitle { "Are you absolutely sure?" }
                    AlertDialogDescription { "This action cannot be undone. It will permanently delete the resource." }
                }
                AlertDialogFooter {
                    AlertDialogCancel {
                        class: "h-9 px-3 rounded-lg border border-border text-sm".to_string(),
                        on_click: move |_| open.set(false),
                        "Cancel"
                    }
                    AlertDialogAction {
                        class: "h-9 px-3 rounded-lg bg-destructive text-destructive-foreground text-sm".to_string(),
                        on_click: move |_| open.set(false),
                        "Delete"
                    }
                }
            }
        }
    }
}
