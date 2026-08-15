//! Combobox — primitive-backed autocomplete input with filterable popup.
//!
//! Wraps `dioxus_primitives::combobox` with shadcn v4 maia styling.
//! The primitive lays out as `Combobox > ComboboxInput + ComboboxList >
//! ComboboxOption*`, replacing the prior hand-rolled cmdk-style
//! Trigger/Content composition. Use `fuzzy_nucleo_filter()` to plug
//! fzf-style ranking into the primitive's `filter` callback.

use architect_story_runtime::story;
use dioxus::prelude::*;
use dioxus_primitives::combobox::{
    Combobox as PrimitiveCombobox, ComboboxEmpty as PrimitiveComboboxEmpty,
    ComboboxInput as PrimitiveComboboxInput, ComboboxItemIndicator,
    ComboboxList as PrimitiveComboboxList, ComboboxOption as PrimitiveComboboxOption,
};
use lucide_dioxus::Check;
use nucleo_matcher::{
    pattern::{CaseMatching, Normalization, Pattern},
    Matcher, Utf32Str,
};

/// Fuzzy-match `query` against any of `candidates`. Returns `Some(score)`
/// for the best-matching candidate, or `None` if nothing matches.
///
/// Uses `nucleo-matcher` so ranking matches fzf/cmdk expectations:
/// subsequence with bonuses for prefix, word-boundary, and consecutive
/// characters. Non-ASCII strings round-trip through a UTF-32 buffer.
pub(super) fn fuzzy_best_score(query: &str, candidates: &[&str]) -> Option<u32> {
    if query.is_empty() {
        return Some(0);
    }
    let mut matcher = Matcher::new(nucleo_matcher::Config::DEFAULT);
    let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
    let mut buf = Vec::new();
    candidates
        .iter()
        .filter_map(|c| {
            let haystack = Utf32Str::new(c, &mut buf);
            pattern.score(haystack, &mut matcher)
        })
        .max()
}

/// Nucleo-backed filter callback for `Combobox::filter`.
/// Returns `true` when the option's text matches the query fzf-style.
pub fn fuzzy_nucleo_filter() -> Callback<(String, String), bool> {
    Callback::new(|(query, text): (String, String)| {
        fuzzy_best_score(&query, &[text.as_str()]).is_some()
    })
}

// ── Data helper ──────────────────────────────────────────────────────────────

/// Data record for a single combobox option. Pair with `for item in items`
/// inside `ComboboxList` to render a list. Carries `keywords` so synonyms
/// / emoji can match a search even when they aren't in the visible label.
#[derive(Clone, PartialEq)]
pub struct ComboboxItemData {
    /// Stable selection value.
    pub value: String,
    /// Visible label. Defaults to `value` if empty.
    pub label: String,
    /// Extra search terms — synonyms, abbreviations, emoji.
    pub keywords: Vec<String>,
    /// When true, the item renders muted and ignores clicks.
    pub disabled: bool,
}

impl ComboboxItemData {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            keywords: Vec::new(),
            disabled: false,
        }
    }

    pub fn keywords<I, S>(mut self, kw: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.keywords = kw.into_iter().map(Into::into).collect();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Text used by the primitive's filter callback. Joins label + value
    /// + keywords so any of them can match.
    pub fn text_value(&self) -> String {
        let label = if self.label.is_empty() {
            self.value.as_str()
        } else {
            self.label.as_str()
        };
        if self.keywords.is_empty() {
            format!("{label} {}", self.value)
        } else {
            format!("{label} {} {}", self.value, self.keywords.join(" "))
        }
    }
}

// ── Combobox root ────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct ComboboxProps {
    /// Currently selected value (two-way bound). Empty = nothing selected.
    pub value: Signal<String>,
    #[props(default)]
    pub on_change: Option<Callback<String>>,
    #[props(default = false)]
    pub disabled: bool,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn Combobox(props: ComboboxProps) -> Element {
    let mut value = props.value;
    let selected: ReadSignal<Option<String>> = use_memo(move || {
        let v = value();
        if v.is_empty() {
            None
        } else {
            Some(v)
        }
    })
    .into();

    rsx! {
        document::Style {
            r#"
                .fts-combobox-option[aria-selected="true"] {{
                    background: var(--accent);
                    color: var(--accent-foreground);
                }}
                .fts-combobox-option:focus,
                .fts-combobox-option[data-focused="true"] {{
                    background: var(--accent);
                    color: var(--accent-foreground);
                }}
                .fts-combobox-option[aria-disabled="true"] {{
                    pointer-events: none;
                    opacity: 0.5;
                }}
            "#
        }
        PrimitiveCombobox::<String> {
            value: Some(selected),
            disabled: props.disabled,
            filter: fuzzy_nucleo_filter(),
            on_value_change: move |next: Option<String>| {
                let next = next.unwrap_or_default();
                value.set(next.clone());
                if let Some(callback) = &props.on_change {
                    callback.call(next);
                }
            },
            class: crate::cn::merge_slice(&["relative inline-block w-full", props.class.as_str()]),
            {props.children}
        }
    }
}

// ── Input ────────────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct ComboboxInputProps {
    #[props(default = "Search...".to_string())]
    pub placeholder: String,
    #[props(default)]
    pub class: String,
}

#[component]
pub fn ComboboxInput(props: ComboboxInputProps) -> Element {
    rsx! {
        PrimitiveComboboxInput {
            placeholder: props.placeholder,
            class: crate::cn::merge(format!(
                "flex h-9 w-full items-center rounded-lg border border-input bg-input/30 px-3 text-sm outline-none transition-colors hover:bg-input/50 focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/50 disabled:cursor-not-allowed disabled:opacity-50 {}",
                props.class
            )),
        }
    }
}

// ── List ─────────────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct ComboboxListProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ComboboxList(props: ComboboxListProps) -> Element {
    rsx! {
        PrimitiveComboboxList {
            class: crate::cn::merge(format!(
                "absolute left-0 top-full z-50 mt-1 max-h-72 w-full overflow-hidden rounded-lg border border-border bg-popover text-popover-foreground shadow-md p-1 {}",
                props.class
            )),
            div {
                class: "max-h-64 overflow-y-auto",
                {props.children}
            }
        }
    }
}

// ── Option ───────────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct ComboboxOptionWrapperProps {
    pub value: String,
    pub index: usize,
    #[props(default = false)]
    pub disabled: bool,
    /// Searchable text. Defaults to `value` if not set.
    #[props(default)]
    pub text_value: Option<String>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ComboboxOption(props: ComboboxOptionWrapperProps) -> Element {
    let text_value: ReadSignal<Option<String>> = Signal::new(Some(
        props.text_value.unwrap_or_else(|| props.value.clone()),
    ))
    .into();
    rsx! {
        PrimitiveComboboxOption::<String> {
            value: props.value,
            text_value,
            index: props.index,
            disabled: props.disabled,
            class: crate::cn::merge(format!(
                "fts-combobox-option relative flex w-full cursor-pointer select-none items-center gap-2 rounded-md px-3 py-2 text-sm outline-none transition-colors hover:bg-accent hover:text-accent-foreground {}",
                props.class
            )),
            span { class: "flex-1", {props.children} }
            ComboboxItemIndicator {
                span { class: "ml-auto flex size-3.5 items-center justify-center",
                    Check { class: "size-4 text-current" }
                }
            }
        }
    }
}

// ── Empty ────────────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct ComboboxEmptyProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn ComboboxEmpty(props: ComboboxEmptyProps) -> Element {
    rsx! {
        PrimitiveComboboxEmpty {
            class: crate::cn::merge(format!(
                "py-6 text-center text-sm text-muted-foreground {}",
                props.class
            )),
            {props.children}
        }
    }
}

// ── Story ────────────────────────────────────────────────────────────────────

#[story(category = "Combobox", name = "combobox default")]
pub fn combobox_default() -> Element {
    let value = use_signal(|| "apple".to_string());
    let items = vec![
        ComboboxItemData::new("apple", "Apple").keywords(["fruit", "red", "🍎"]),
        ComboboxItemData::new("banana", "Banana").keywords(["fruit", "yellow", "🍌"]),
        ComboboxItemData::new("cherry", "Cherry").keywords(["fruit", "red", "🍒"]),
        ComboboxItemData::new("durian", "Durian").keywords(["fruit", "stinky"]),
    ];
    rsx! {
        div { class: "p-6 bg-background text-foreground max-w-xs",
            Combobox { value,
                ComboboxInput { placeholder: "Search fruit...".to_string() }
                ComboboxList {
                    ComboboxEmpty { "No fruit matches that search." }
                    for (idx, item) in items.iter().enumerate() {
                        ComboboxOption {
                            key: "{item.value}",
                            value: item.value.clone(),
                            text_value: Some(item.text_value()),
                            index: idx,
                            disabled: item.disabled,
                            {if item.label.is_empty() { item.value.clone() } else { item.label.clone() }}
                        }
                    }
                }
            }
        }
    }
}
