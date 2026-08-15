//! Inline edit — double-click-to-rename toggle.
//!
//! Switches between a display button and a text input. Commits on Enter or
//! blur, cancels on Escape. Used for renaming scenes, presets, songs, etc.

use dioxus::prelude::*;
use architect_story_runtime::story;

/// A text element that switches to an input on double-click for inline renaming.
///
/// ```rust,ignore
/// InlineEdit {
///     value: "Scene A",
///     on_commit: move |new_name: String| {
///         rename_scene(scene_id, new_name);
///     },
/// }
/// ```
#[derive(Props, Clone, PartialEq)]
pub struct InlineEditProps {
    /// The current display text.
    pub value: String,

    /// Called with the new text when the user commits (Enter or blur).
    /// Not called if the text is unchanged or empty.
    pub on_commit: Callback<String>,

    /// Whether the item is visually selected/active.
    #[props(default = false)]
    pub selected: bool,

    /// Called when editing state changes. Useful for suppressing keyboard
    /// shortcuts while the input is focused.
    #[props(default)]
    pub on_editing_change: Option<Callback<bool>>,

    /// Placeholder text shown in the input when empty.
    #[props(default = "Enter name...".to_string())]
    pub placeholder: String,

    /// Extra CSS classes on the outer element (both button and input states).
    #[props(default)]
    pub class: String,
}

#[component]
pub fn InlineEdit(props: InlineEditProps) -> Element {
    let mut is_editing = use_signal(|| false);
    let mut draft = use_signal(String::new);

    let mut enter_edit = {
        let value = props.value.clone();
        move || {
            draft.set(value.clone());
            is_editing.set(true);
            if let Some(cb) = &props.on_editing_change {
                cb.call(true);
            }
        }
    };

    let cancel_edit = move || {
        is_editing.set(false);
        if let Some(cb) = &props.on_editing_change {
            cb.call(false);
        }
    };

    let mut commit_edit = {
        let original = props.value.clone();
        move || {
            let new_val = draft().trim().to_string();
            is_editing.set(false);
            if let Some(cb) = &props.on_editing_change {
                cb.call(false);
            }
            if !new_val.is_empty() && new_val != original {
                props.on_commit.call(new_val);
            }
        }
    };

    if is_editing() {
        let mut commit_key = commit_edit.clone();
        let mut cancel_key = cancel_edit;

        rsx! {
            input {
                class: crate::cn::merge(format!(
                    "px-2 py-0.5 text-xs text-foreground bg-secondary border border-input rounded outline-none {}",
                    props.class
                )),
                value: "{draft}",
                placeholder: "{props.placeholder}",
                autofocus: true,
                onmounted: move |elem| async move {
                    let _ = elem.set_focus(true).await;
                },
                oninput: move |e| draft.set(e.value()),
                onkeydown: move |e: KeyboardEvent| {
                    e.stop_propagation();
                    if e.key() == Key::Enter {
                        commit_key();
                    }
                    if e.key() == Key::Escape {
                        cancel_key();
                    }
                },
                onfocusout: move |_| commit_edit(),
            }
        }
    } else {
        let active_class = if props.selected {
            "bg-accent text-accent-foreground font-medium"
        } else {
            "text-muted-foreground hover:text-foreground hover:bg-accent/50"
        };

        rsx! {
            button {
                class: crate::cn::merge(format!(
                    "px-2.5 py-1 text-xs rounded transition-colors {active_class} {}",
                    props.class
                )),
                ondoubleclick: move |_| enter_edit(),
                "{props.value}"
            }
        }
    }
}

#[story(category = "InlineEdit", name = "inline_edit_default")]
pub fn inline_edit_default() -> Element {
    let mut name = use_signal(|| "Scene A".to_string());
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center gap-3",
            InlineEdit {
                value: name(),
                on_commit: move |new_val: String| name.set(new_val),
            }
            span { class: "text-xs text-muted-foreground", "(double-click to edit)" }
        }
    }
}
