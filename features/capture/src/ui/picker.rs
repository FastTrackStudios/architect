//! The property picker: a searchable list for one property (#445).
//!
//! Ported from Task (`task:features/task/task-ui/src/views/picker.rs`):
//! type to filter (a whole-label prefix first, then a word's prefix, then
//! anywhere), ↑ / ↓ / Enter, Esc closes; digits pick a numbered row while
//! nothing is typed; `multi` keeps it open and shows checkboxes; with
//! `on_create`, a query no option has offers to make one. A sheet over
//! the keyboard on a phone, a dropdown under its chip on a desk.

// A cursor over a short list: wrapping arithmetic on small counts.
#![allow(clippy::arithmetic_side_effects)]

use dioxus::prelude::*;
use lucide_dioxus as lc;

use super::style::{self, CaptureCss, FIELD_RESET, RESET, TRUNCATE, use_look, wash};

/// One choice: its id (what `on_pick` gets), its label, and an icon or a
/// colour dot before it.
#[derive(Clone, PartialEq)]
pub struct PickOption {
    pub id: String,
    pub label: String,
    pub icon: Option<Element>,
    pub dot: Option<String>,
}

impl PickOption {
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            dot: None,
        }
    }

    /// With an icon before the label.
    #[must_use]
    pub fn icon(mut self, icon: Element) -> Self {
        self.icon = Some(icon);
        self
    }

    /// With a colour dot before the label.
    #[must_use]
    pub fn dot(mut self, color: impl Into<String>) -> Self {
        self.dot = Some(color.into());
        self
    }
}

/// How well `query` matches `label`: 0 its start, 1 a word's start, 2
/// anywhere, `None` not at all (and 0 for nothing typed).
#[must_use]
pub fn match_rank(query: &str, label: &str) -> Option<u8> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Some(0);
    }
    let l = label.to_lowercase();
    if l.starts_with(&q) {
        Some(0)
    } else if l
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(&q))
    {
        Some(1)
    } else if l.contains(&q) {
        Some(2)
    } else {
        None
    }
}

/// The options `query` keeps, best first, each with its place in
/// `options` (what its number says).
#[must_use]
pub fn filtered(options: &[PickOption], query: &str) -> Vec<(usize, PickOption)> {
    let mut hits: Vec<(u8, usize, PickOption)> = options
        .iter()
        .enumerate()
        .filter_map(|(i, o)| match_rank(query, &o.label).map(|r| (r, i, o.clone())))
        .collect();
    hits.sort_by_key(|(r, i, _)| (*r, *i));
    hits.into_iter().map(|(_, i, o)| (i, o)).collect()
}

/// The picker. Nothing while closed.
#[allow(clippy::too_many_lines)]
#[component]
pub fn PropertyPicker(
    open: bool,
    title: String,
    /// Its key, shown on a desk (`S`).
    #[props(default)]
    hotkey: Option<String>,
    options: Vec<PickOption>,
    #[props(default)] selected: Vec<String>,
    #[props(default)] multi: bool,
    /// Rows numbered from this (`None`: not numbered).
    #[props(default = Some(1))]
    number_from: Option<u8>,
    #[props(default)] on_create: Option<EventHandler<String>>,
    on_pick: EventHandler<String>,
    on_close: EventHandler<()>,
) -> Element {
    let l = use_look();
    let phone = style::phone();
    let mut query = use_signal(String::new);
    let mut cursor = use_signal(|| 0usize);
    let mut was_open = use_signal(|| false);
    if open != *was_open.peek() {
        was_open.set(open);
        if open {
            query.set(String::new());
            cursor.set(0);
        }
    }
    if !open {
        return rsx! {};
    }

    let rows = filtered(&options, &query.read());
    let q = query.read().trim().to_owned();
    let can_create = on_create.is_some()
        && !q.is_empty()
        && !options.iter().any(|o| o.label.eq_ignore_ascii_case(&q));
    let total = rows.len() + usize::from(can_create);
    let at = (*cursor.read()).min(total.saturating_sub(1));

    let pick = {
        let rows = rows.clone();
        move |i: usize| {
            if let Some((_, o)) = rows.get(i) {
                on_pick.call(o.id.clone());
                if !multi {
                    on_close.call(());
                }
            } else if can_create && let Some(create) = on_create {
                create.call(query.peek().trim().to_owned());
                query.set(String::new());
                if !multi {
                    on_close.call(());
                }
            }
        }
    };
    let mut pick_kb = pick.clone();
    let options_kb = options;
    let on_key = move |e: KeyboardEvent| match e.key() {
        Key::Named(NamedKey::Escape) => {
            e.prevent_default();
            e.stop_propagation();
            on_close.call(());
        }
        Key::Named(NamedKey::ArrowDown) => {
            e.prevent_default();
            if total > 0 {
                cursor.set((at + 1) % total);
            }
        }
        Key::Named(NamedKey::ArrowUp) => {
            e.prevent_default();
            if total > 0 {
                cursor.set((at + total - 1) % total);
            }
        }
        Key::Named(NamedKey::Enter) => {
            e.prevent_default();
            e.stop_propagation();
            pick_kb(at);
        }
        Key::Character(c) if query.peek().is_empty() => {
            if let (Some(from), Ok(n)) = (number_from, c.parse::<usize>())
                && let Some(idx) = n.checked_sub(usize::from(from))
                && let Some(o) = options_kb.get(idx)
            {
                e.prevent_default();
                on_pick.call(o.id.clone());
                if !multi {
                    on_close.call(());
                }
            }
        }
        _ => {}
    };

    // A phone: a sheet over the keyboard, under a scrim. A desk: a
    // dropdown under the chips.
    let scrim = if phone {
        format!(
            "position: absolute; top: 0; left: 0; right: 0; bottom: 0; z-index: 40; background: {};",
            l.scrim
        )
    } else {
        "position: fixed; top: 0; left: 0; right: 0; bottom: 0; z-index: 40;".to_owned()
    };
    let panel = if phone {
        format!(
            "position: absolute; left: 0; right: 0; bottom: 0; z-index: 50; display: flex; flex-direction: column; \
             max-height: 70%; overflow: hidden; border-radius: 30px 30px 0 0; background: {}; color: {}; \
             padding-bottom: {}px; \
             box-shadow: 0 -12px 40px -12px rgba(0,0,0,0.6), inset 0 1px 0 rgba(255,255,255,0.07);",
            l.sheet,
            l.ink,
            style::insets().bottom.max(12.0)
        )
    } else {
        format!(
            "position: absolute; left: 0; top: 100%; margin-top: 4px; z-index: 50; display: flex; flex-direction: column; \
             max-height: 320px; width: 256px; overflow: hidden; border-radius: 8px; border: 1px solid {}; \
             background: {}; color: {}; box-shadow: {};",
            l.rule, l.sheet, l.ink, l.lift
        )
    };
    let (row_h, row_text) = if phone { (44, 15) } else { (32, 13) };
    let pad = if phone { 12 } else { 8 };
    let row = move |active: bool, l: &style::Look| {
        format!(
            "{RESET} display: flex; min-height: {row_h}px; width: 100%; align-items: center; gap: 10px; border-radius: 6px; \
             padding: 0 10px; text-align: left; font-size: {row_text}px; color: {}; background: {};",
            l.ink,
            if active {
                l.hover.as_str()
            } else {
                "transparent"
            }
        )
    };
    rsx! {
        CaptureCss {}
        div { style: "{scrim}", onclick: move |_| on_close.call(()) }
        div {
            style: "{panel}",
            role: "listbox",
            "aria-label": "{title}",
            onclick: move |e| e.stop_propagation(),
            div { style: "display: flex; align-items: center; gap: 8px; border-bottom: 1px solid {wash(&l.rule, 70)}; padding: 0 12px;",
                input {
                    r#type: "text",
                    class: "cap-in",
                    style: "{FIELD_RESET} --cap-ph: {l.ink_2}; min-width: 0; flex: 1; padding: {pad}px 0; \
                            font-size: {row_text}px; color: {l.ink};",
                    placeholder: "{title}",
                    value: "{query}",
                    autofocus: true,
                    onmounted: move |e| async move {
                        let _ = e.set_focus(true).await;
                    },
                    oninput: move |e| {
                        query.set(e.value());
                        cursor.set(0);
                    },
                    onkeydown: on_key,
                }
                if let (Some(k), false) = (hotkey.as_ref(), phone) {
                    kbd { style: "flex-shrink: 0; border-radius: 4px; border: 1px solid {wash(&l.rule, 60)}; padding: 0 6px; \
                                  font-family: monospace; font-size: 10px; color: {l.ink_2};",
                        "{k}"
                    }
                }
            }
            div { style: "min-height: 0; flex: 1; overflow-y: auto; padding: 4px;",
                if rows.is_empty() && !can_create {
                    p { style: "margin: 0; padding: 8px 10px; font-size: 13px; color: {l.ink_2};", "No matches" }
                }
                for (n , (idx , o)) in rows.iter().cloned().enumerate() {
                    {
                        let chosen = selected.contains(&o.id);
                        let mut pick = pick.clone();
                        let number = number_from.map(|from| idx + usize::from(from)).filter(|n| *n < 10);
                        rsx! {
                            button {
                                key: "{o.id}",
                                r#type: "button",
                                role: "option",
                                "aria-selected": "{chosen}",
                                style: "{row(n == at, &l)}",
                                onmouseenter: move |_| cursor.set(n),
                                onclick: move |_| pick(n),
                                if multi {
                                    span { style: if chosen {
                                            format!("display: flex; width: 16px; height: 16px; flex-shrink: 0; align-items: center; justify-content: center; border-radius: 4px; border: 1px solid {}; background: {}; color: {};", l.brand, l.brand, l.brand_ink)
                                        } else {
                                            format!("display: flex; width: 16px; height: 16px; flex-shrink: 0; align-items: center; justify-content: center; border-radius: 4px; border: 1px solid {}; box-sizing: border-box;", l.rule)
                                        },
                                        if chosen { lc::Check { size: 11 } }
                                    }
                                }
                                if let Some(icon) = o.icon.clone() {
                                    span { style: "display: flex; width: 16px; height: 16px; flex-shrink: 0; align-items: center; justify-content: center; color: {l.ink_2};", {icon} }
                                } else if let Some(c) = o.dot.as_ref() {
                                    span { style: "width: 10px; height: 10px; flex-shrink: 0; border-radius: 999px; background: {c};" }
                                }
                                span { style: "min-width: 0; flex: 1; {TRUNCATE}", "{o.label}" }
                                if chosen && !multi {
                                    span { style: "flex-shrink: 0; display: flex; color: {l.ink};", lc::Check { size: 14 } }
                                }
                                if let Some(n) = number {
                                    span { style: "width: 12px; flex-shrink: 0; text-align: right; font-family: monospace; font-size: 11px; color: {l.ink_2};", "{n}" }
                                }
                            }
                        }
                    }
                }
                if can_create {
                    {
                        let n = rows.len();
                        let mut pick = pick;
                        rsx! {
                            button {
                                r#type: "button",
                                style: "{row(n == at, &l)}",
                                onmouseenter: move |_| cursor.set(n),
                                onclick: move |_| pick(n),
                                span { style: "display: flex; width: 16px; height: 16px; flex-shrink: 0; align-items: center; justify-content: center; color: {l.ink_2};",
                                    lc::Plus { size: 14 }
                                }
                                span { style: "min-width: 0; flex: 1; {TRUNCATE}", "Create “{q}”" }
                            }
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(labels: &[&str]) -> Vec<PickOption> {
        labels.iter().map(|l| PickOption::new(*l, *l)).collect()
    }

    #[test]
    fn typing_keeps_matches_prefix_first_with_their_numbers() {
        let o = opts(&["No priority", "Urgent", "High", "Medium", "Low"]);
        let me: Vec<usize> = filtered(&o, "me").into_iter().map(|(i, _)| i).collect();
        assert_eq!(me, vec![3], "Medium, still addressed as row 3");
        let low: Vec<usize> = filtered(&o, "lo").into_iter().map(|(i, _)| i).collect();
        assert_eq!(low, vec![4]);
        assert_eq!(filtered(&o, "pri").len(), 1);
    }

    #[test]
    fn nothing_typed_keeps_everything_in_order() {
        let o = opts(&["Todo", "In Progress", "Done"]);
        let all: Vec<usize> = filtered(&o, "").into_iter().map(|(i, _)| i).collect();
        assert_eq!(all, vec![0, 1, 2]);
    }

    #[test]
    fn whole_label_prefix_beats_a_word_prefix_beats_anywhere() {
        assert_eq!(match_rank("in", "In Progress"), Some(0));
        assert_eq!(match_rank("pro", "In Progress"), Some(1));
        assert_eq!(match_rank("gres", "In Progress"), Some(2));
        assert_eq!(match_rank("zz", "In Progress"), None);
    }
}
