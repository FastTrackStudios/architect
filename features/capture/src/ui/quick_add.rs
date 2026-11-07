//! Quick add: one line, read as it is typed, captured on Enter (#445).
//!
//! Ported from Task (`task:features/task/task-ui/src/views/quick_add.rs`):
//! the inline bar with a live preview of what the parser found ("Will
//! add: Fri, Nov 6 · #mix · @studio"), and a project to file it in. It
//! captures through `CAPTURE_ADD`, into the `task` target when the Tasks
//! extension is there (the inbox otherwise), so a list's quick add and the
//! composer make the same thing.

use architect::action::ActionCaller;
use dioxus::prelude::*;
use lucide_dioxus as lc;
use serde_json::{Map, json};

use super::style::{self, CaptureCss, FIELD_RESET, RESET, use_look, wash};
use super::{CaptureUi, facet_of};
use crate::actions::{AddArgs, Capturer};
use crate::target::Target;

/// What the parser found in `line`, as the preview names it: (kind,
/// label).
#[must_use]
pub fn parsed_bits(line: &str, targets: &[Target]) -> Vec<(&'static str, String)> {
    let d = Capturer::parse(line, targets);
    let mut bits = Vec::new();
    if let Some(day) = d.due {
        bits.push(("due", day.format("%a, %b %-d").to_string()));
    }
    if let Some(r) = d.repeat.as_ref() {
        bits.push(("repeat", super::repeat_label(&r.rule)));
    }
    for p in &d.projects {
        bits.push(("project", crate::vocabulary::bare_name(p)));
    }
    for tag in &d.tags {
        bits.push(("tag", format!("#{tag}")));
    }
    for c in &d.contexts {
        bits.push(("context", format!("@{}", c.trim_start_matches('@'))));
    }
    if let Some(p) = d.priority.as_ref().filter(|p| *p != "normal") {
        bits.push(("priority", p.clone()));
    }
    for (k, v) in &d.extra {
        bits.push(("extra", format!("{k}: {v}")));
    }
    bits
}

/// The quick-add bar. `projects` offers a project to file it in.
#[allow(clippy::too_many_lines)]
#[component]
pub fn QuickAdd(
    capture: CaptureUi,
    #[props(default)] projects: Vec<String>,
    #[props(default)] autofocus: bool,
    /// What the bar says before anything is typed.
    #[props(default = "Add a task — \"Mix vocals friday #trackone\"".to_owned())]
    placeholder: String,
    /// After each capture, its answer (the made note's path).
    #[props(default)]
    on_added: Option<EventHandler<String>>,
) -> Element {
    let l = use_look();
    let phone = style::phone();
    let mut value = use_signal(String::new);
    let mut chosen_project = use_signal(String::new);
    let mut focused = use_signal(|| false);
    let mut said = use_signal(|| None::<String>);
    let targets = use_resource(move || {
        let cap = capture.capturer();
        async move { cap.targets(&ActionCaller::app()).await }
    });
    let all = targets
        .read()
        .clone()
        .unwrap_or_else(|| vec![Target::note()]);
    let has_task = all.iter().any(|t| t.id == "task");

    let mut submit = move || {
        let text = value.peek().trim().to_owned();
        if text.is_empty() {
            return;
        }
        let mut props = Map::new();
        let chosen = chosen_project.peek().clone();
        if !chosen.is_empty() {
            props.insert("projects".to_owned(), json!([format!("[[{chosen}]]")]));
        }
        let args = AddArgs {
            text,
            properties: (!props.is_empty()).then(|| facet_of(&props)).flatten(),
            target: has_task.then(|| "task".to_owned()),
            source: Some("ui".to_owned()),
        };
        let cap = capture.capturer();
        value.set(String::new());
        spawn(async move {
            match cap.capture(&ActionCaller::app(), args).await {
                Ok(v) => {
                    capture.changed();
                    said.set(None);
                    if let Some(on) = on_added {
                        on.call(
                            v.get("path")
                                .and_then(|p| p.as_str())
                                .unwrap_or_default()
                                .to_owned(),
                        );
                    }
                }
                Err(e) => said.set(Some(e)),
            }
        });
    };

    let line = value.read().clone();
    let bits = if line.trim().is_empty() {
        Vec::new()
    } else {
        parsed_bits(&line, &all)
    };
    let ring = if focused() {
        format!(
            "border: 1px solid {}; background: {}; box-shadow: 0 0 0 2px {};",
            l.rule, l.sheet, l.brand_soft
        )
    } else {
        format!(
            "border: 1px solid {}; background: transparent;",
            wash(&l.rule, 60)
        )
    };
    let text_size = if phone { 16 } else { 14 };
    rsx! {
        CaptureCss {}
        div { style: "display: flex; flex-direction: column; gap: 6px; color: {l.ink}; font-family: {l.font};",
            form {
                style: "display: flex; align-items: center; gap: 8px; border-radius: 8px; padding: 6px 12px; {ring}",
                onsubmit: move |e: Event<FormData>| {
                    e.prevent_default();
                    submit();
                },
                div { style: "display: flex; width: 24px; height: 24px; flex-shrink: 0; align-items: center; justify-content: center; \
                              border-radius: 6px; background: {l.brand_soft}; color: {l.brand};",
                    lc::Plus { size: 14 }
                }
                input {
                    r#type: "text",
                    class: "cap-in",
                    value: "{value}",
                    "aria-label": "New task",
                    placeholder: "{placeholder}",
                    style: "{FIELD_RESET} --cap-ph: {l.ink_2}; min-width: 0; flex: 1; font-size: {text_size}px; color: {l.ink};",
                    oninput: move |e| value.set(e.value()),
                    onfocus: move |_| focused.set(true),
                    onblur: move |_| focused.set(false),
                    onkeydown: move |e: KeyboardEvent| {
                        if e.key() == Key::Named(NamedKey::Enter) {
                            e.prevent_default();
                            submit();
                        }
                    },
                    onmounted: move |e| async move {
                        if autofocus {
                            let _ = e.set_focus(true).await;
                        }
                    },
                }
                if !projects.is_empty() {
                    select {
                        "aria-label": "Project",
                        style: "max-width: 35%; flex-shrink: 0; border-radius: 6px; border: 1px solid {wash(&l.rule, 60)}; background: transparent; \
                                padding: 4px 6px; font-size: 12px; color: {l.ink_2}; font-family: inherit;",
                        value: "{chosen_project}",
                        onchange: move |e| chosen_project.set(e.value()),
                        option { value: "", selected: chosen_project.read().is_empty(), "No project" }
                        for name in projects.iter() {
                            option { key: "{name}", value: "{name}", selected: *chosen_project.read() == *name, "{name}" }
                        }
                    }
                }
                if !phone {
                    kbd { style: "border-radius: 4px; border: 1px solid {l.rule}; background: {wash(&l.ink, 5)}; padding: 2px 6px; font-size: 12px; color: {l.ink_2};",
                        "⏎"
                    }
                } else {
                    button {
                        r#type: "submit",
                        class: "cap-btn",
                        style: "{RESET} flex-shrink: 0; border-radius: 6px; background: {l.brand}; color: {l.brand_ink}; padding: 6px 12px; \
                                font-size: 14px; font-weight: 500;",
                        disabled: value.read().trim().is_empty(),
                        "Add"
                    }
                }
            }
            if !bits.is_empty() {
                div { style: "display: flex; flex-wrap: wrap; align-items: center; gap: 6px; padding: 0 4px; font-size: 12px; color: {l.ink_2};",
                    "aria-live": "polite",
                    span { "Will add:" }
                    for (kind , label) in bits {
                        span { key: "{kind}{label}", style: "display: inline-flex; align-items: center; gap: 4px; border-radius: 999px; \
                                                             background: {wash(&l.ink, 8)}; padding: 2px 8px; color: {wash(&l.ink, 85)};",
                            match kind {
                                "due" => rsx! { lc::CalendarDays { size: 11 } },
                                "repeat" => rsx! { lc::Repeat { size: 11 } },
                                "project" => rsx! { lc::Hash { size: 11 } },
                                "priority" => rsx! { lc::Flag { size: 11 } },
                                _ => rsx! {},
                            }
                            "{label}"
                        }
                    }
                }
            }
            if let Some(e) = said() {
                div { style: "padding: 0 4px; font-size: 12px; color: {l.danger};", "{e}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parsed_bits;

    #[test]
    fn the_preview_names_what_capture_found() {
        let bits = parsed_bits("Mix vocals tomorrow #trackone @studio", &[]);
        assert!(bits.iter().any(|(k, _)| *k == "due"), "{bits:?}");
        assert!(
            bits.iter().any(|(k, v)| *k == "tag" && v == "#trackone"),
            "{bits:?}"
        );
        assert!(
            bits.iter().any(|(k, v)| *k == "context" && v == "@studio"),
            "{bits:?}"
        );
    }

    #[test]
    fn a_plain_title_previews_nothing() {
        let bits = parsed_bits("Call the venue", &[]);
        assert!(bits.is_empty(), "{bits:?}");
    }
}
