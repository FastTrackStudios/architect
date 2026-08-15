//! InputOtp — shadcn v4 maia style one-time password input.

use architect_story_runtime::story;
use dioxus::prelude::*;

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct InputOtpContext {
    value: Signal<String>,
    focused: Signal<bool>,
}

// ---------------------------------------------------------------------------
// InputOtp
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct InputOtpProps {
    /// Total number of OTP characters.
    #[props(default = 6)]
    pub length: usize,

    /// Two-way bound value string.
    pub value: Signal<String>,

    /// Called when all slots are filled.
    #[props(default)]
    pub on_complete: Option<Callback<String>>,

    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// Root OTP input. Renders a hidden `<input>` for keyboard capture and provides
/// context so child `InputOtpSlot` elements can display individual characters.
#[component]
pub fn InputOtp(props: InputOtpProps) -> Element {
    let mut value = props.value;
    let length = props.length;
    let on_complete = props.on_complete;
    let mut focused = use_signal(|| false);

    use_context_provider(|| InputOtpContext { value, focused });

    rsx! {
        div {
            class: crate::cn::merge_slice(&["relative flex items-center gap-2", props.class.as_str()]),
            onclick: move |_| {
                // Clicking anywhere in the component focuses the hidden input.
                // The JS eval below focuses the sibling input element.
            },

            // Hidden input that captures actual keyboard events.
            input {
                r#type: "text",
                inputmode: "numeric",
                autocomplete: "one-time-code",
                maxlength: "{length}",
                value: "{value}",
                class: "absolute inset-0 w-full h-full opacity-0 cursor-pointer",
                autofocus: true,
                onmounted: move |elem| async move {
                    let _ = elem.set_focus(true).await;
                },
                onfocus: move |_| { focused.set(true); },
                onblur: move |_| { focused.set(false); },
                oninput: move |evt| {
                    let raw: String = evt
                        .value()
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .take(length)
                        .collect();
                    value.set(raw.clone());
                    if raw.len() == length {
                        if let Some(cb) = &on_complete {
                            cb.call(raw);
                        }
                    }
                },
            }

            // Visual slot rendering is done by children (InputOtpGroup / InputOtpSlot).
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// InputOtpGroup
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct InputOtpGroupProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Visual grouping wrapper for OTP slots (e.g. groups of 3).
#[component]
pub fn InputOtpGroup(props: InputOtpGroupProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex items-center", props.class.as_str()]),
            {props.children}
        }
    }
}

// ---------------------------------------------------------------------------
// InputOtpSlot
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct InputOtpSlotProps {
    /// Zero-based index of this slot.
    pub index: usize,
    #[props(default)]
    pub class: String,
}

/// Displays a single character slot. Reads the character at `index` from context.
#[component]
pub fn InputOtpSlot(props: InputOtpSlotProps) -> Element {
    let ctx: InputOtpContext = use_context();
    let val = ctx.value.read();
    let ch = val.chars().nth(props.index);
    let is_focused = *ctx.focused.read() && val.len() == props.index;

    let focus_class = if is_focused {
        "border-ring ring-ring/50 ring-[3px]"
    } else {
        ""
    };

    rsx! {
        div {
            class: crate::cn::merge(format!(
                "relative flex items-center justify-center size-9 border-y border-r border-input bg-input/30 text-sm transition-all first:rounded-l-lg first:border-l last:rounded-r-lg {focus_class} {}",
                props.class
            )),
            if let Some(c) = ch {
                span { "{c}" }
            } else if is_focused {
                span { class: "animate-pulse bg-foreground h-4 w-px" }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// InputOtpSeparator
// ---------------------------------------------------------------------------

#[derive(Props, Clone, PartialEq)]
pub struct InputOtpSeparatorProps {
    #[props(default)]
    pub class: String,
}

/// A dash separator between OTP groups.
#[component]
pub fn InputOtpSeparator(props: InputOtpSeparatorProps) -> Element {
    rsx! {
        span {
            class: crate::cn::merge_slice(&["flex items-center text-muted-foreground px-1", props.class.as_str()]),
            "\u{2014}"
        }
    }
}

/// Default 6-digit OTP input split into two groups of 3.
#[story(category = "InputOtp", name = "default")]
pub fn input_otp_default() -> Element {
    let value = use_signal(String::new);
    rsx! {
        div { class: "p-6 bg-background text-foreground",
            InputOtp { value, length: 6,
                InputOtpGroup {
                    InputOtpSlot { index: 0 }
                    InputOtpSlot { index: 1 }
                    InputOtpSlot { index: 2 }
                }
                InputOtpSeparator {}
                InputOtpGroup {
                    InputOtpSlot { index: 3 }
                    InputOtpSlot { index: 4 }
                    InputOtpSlot { index: 5 }
                }
            }
        }
    }
}
