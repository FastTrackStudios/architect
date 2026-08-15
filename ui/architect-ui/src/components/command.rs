//! Command palette — shadcn v4 maia style (Cmd+K dialog).

use architect_story_runtime::story;
use dioxus::prelude::*;

// ── Context ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
struct CommandContext {
    on_close: Option<Callback<()>>,
    /// Live search text — `CommandInput` writes here, `CommandItem`
    /// reads to fuzzy-filter itself.
    search_query: Signal<String>,
}

// ── CommandDialog ───────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct CommandDialogProps {
    /// Whether the dialog is visible.
    #[props(default = false)]
    pub open: bool,

    /// Called when the user dismisses the dialog.
    #[props(default)]
    pub on_close: Option<Callback<()>>,

    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// shadcn v4 maia: command palette dialog with overlay
#[component]
pub fn CommandDialog(props: CommandDialogProps) -> Element {
    let search_query = use_signal(String::new);
    use_context_provider(|| CommandContext {
        on_close: props.on_close,
        search_query,
    });

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
        // Content. `tabindex=-1` + autofocus lets the wrapper receive
        // keystrokes so Escape works even before any inner input is
        // focused. Inner inputs propagate keydown up to this handler
        // unless they explicitly stop it.
        div {
            class: crate::cn::merge(format!(
                "fixed left-1/2 top-1/2 -translate-x-1/2 -translate-y-1/2 z-50 w-full max-w-md rounded-xl bg-popover text-popover-foreground border border-border shadow-md p-0 overflow-hidden outline-none {}",
                props.class
            )),
            tabindex: "-1",
            onclick: move |e| e.stop_propagation(),
            onkeydown: close_on_escape,
            {props.children}
        }
    }
}

// ── CommandInput ────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct CommandInputProps {
    /// Two-way bound search text.
    pub value: Signal<String>,

    #[props(default = "Search...".to_string())]
    pub placeholder: String,

    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: search input at top of command palette
#[component]
pub fn CommandInput(props: CommandInputProps) -> Element {
    let mut value = props.value;
    let mut ctx: CommandContext = use_context();

    // Mirror the user-supplied value Signal into the context so that
    // descendant `CommandItem`s can fuzzy-filter against it without
    // each callsite plumbing the signal through manually.
    use_effect(move || {
        let v = value.read().clone();
        ctx.search_query.set(v);
    });

    rsx! {
        div {
            class: crate::cn::merge_slice(&["flex items-center border-b border-border px-3", props.class.as_str()]),
            // Search icon
            svg {
                class: "size-4 opacity-50 shrink-0",
                xmlns: "http://www.w3.org/2000/svg",
                width: "24",
                height: "24",
                view_box: "0 0 24 24",
                fill: "none",
                stroke: "currentColor",
                stroke_width: "2",
                stroke_linecap: "round",
                stroke_linejoin: "round",
                path { d: "m21 21-4.3-4.3" }
                circle { cx: "11", cy: "11", r: "8" }
            }
            input {
                class: "flex-1 h-11 bg-transparent text-sm placeholder:text-muted-foreground focus:outline-none ml-2",
                r#type: "text",
                placeholder: "{props.placeholder}",
                value: "{value.read()}",
                oninput: move |e| value.set(e.value()),
                autofocus: true,
                onmounted: move |elem| async move {
                    let _ = elem.set_focus(true).await;
                },
            }
        }
    }
}

// ── CommandList ──────────────────────────────────────────────────────────────

/// Data record for a single command palette item. Used with the
/// `CommandList { items: ... }` data-driven API to enable fuzzy-score
/// sorting, group hiding, and automatic empty-state handling.
#[derive(Clone, PartialEq)]
pub struct CommandItemData {
    /// Stable selection value. Fuzzy-matched against the search query.
    pub value: String,
    /// Visible label. Defaults to `value` if empty.
    pub label: String,
    /// Synonyms / abbreviations / emoji also matched against the query.
    pub keywords: Vec<String>,
    /// Optional group heading. Items with the same `group` cluster
    /// together; items with `None` render before any grouped items.
    /// Groups with no surviving items after filtering disappear.
    pub group: Option<String>,
    /// Right-aligned hint text (e.g. "⌘P").
    pub shortcut: Option<String>,
    /// Fired when the item is clicked. The `CommandDialog` close
    /// callback also fires automatically.
    pub on_select: Option<Callback<()>>,
    /// When true, the item renders muted and ignores clicks.
    pub disabled: bool,
}

impl CommandItemData {
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            keywords: Vec::new(),
            group: None,
            shortcut: None,
            on_select: None,
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

    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn on_select(mut self, cb: Callback<()>) -> Self {
        self.on_select = Some(cb);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct CommandListProps {
    /// Data-driven items. When `Some`, items are fuzzy-filtered AND
    /// sorted by score, grouped under their `group`, and `empty` is
    /// rendered when nothing matches. Pass `None` to fall back to the
    /// children-based composition.
    #[props(default)]
    pub items: Option<Vec<CommandItemData>>,

    /// Element shown when `items` is `Some` and no item matches.
    #[props(default)]
    pub empty: Option<Element>,

    #[props(default)]
    pub class: String,

    #[props(default)]
    pub children: Element,
}

/// shadcn v4 maia: scrollable results container
#[component]
pub fn CommandList(props: CommandListProps) -> Element {
    let ctx: CommandContext = use_context();
    rsx! {
        div {
            class: crate::cn::merge_slice(&["max-h-72 overflow-y-auto p-1", props.class.as_str()]),
            if let Some(items) = props.items.as_ref() {
                {render_command_items(items, ctx, props.empty.clone())}
            } else {
                {props.children}
            }
        }
    }
}

/// Render the data-driven `items` path: fuzzy-filter, sort by score
/// within each group, drop empty groups, render `empty` when nothing
/// matches at all.
fn render_command_items(
    items: &[CommandItemData],
    ctx: CommandContext,
    empty: Option<Element>,
) -> Element {
    let query = ctx.search_query.read().clone();

    // Score every item; drop non-matches.
    let scored: Vec<(u32, &CommandItemData)> = items
        .iter()
        .filter_map(|item| {
            let mut candidates: Vec<&str> = vec![item.value.as_str()];
            candidates.extend(item.keywords.iter().map(|s| s.as_str()));
            if !item.label.is_empty() && item.label != item.value {
                candidates.push(item.label.as_str());
            }
            super::combobox::fuzzy_best_score(&query, &candidates).map(|s| (s, item))
        })
        .collect();

    if scored.is_empty() {
        return rsx! {
            if let Some(e) = empty {
                {e}
            } else {
                div {
                    class: "py-6 text-center text-sm text-muted-foreground",
                    "No results."
                }
            }
        };
    }

    // Bucket by group while remembering the order each group first
    // appeared. Within a group, sort by descending score.
    let mut groups_in_order: Vec<Option<String>> = Vec::new();
    let mut buckets: std::collections::HashMap<Option<String>, Vec<(u32, &CommandItemData)>> =
        std::collections::HashMap::new();
    for (score, item) in scored {
        let key = item.group.clone();
        if !buckets.contains_key(&key) {
            groups_in_order.push(key.clone());
        }
        buckets.entry(key).or_default().push((score, item));
    }
    for v in buckets.values_mut() {
        v.sort_by(|a, b| b.0.cmp(&a.0));
    }

    rsx! {
        for (gi, group_key) in groups_in_order.iter().enumerate() {
            {
                let bucket = buckets.remove(group_key).unwrap_or_default();
                let heading = group_key.clone();
                rsx! {
                    div {
                        key: "{gi}",
                        class: "overflow-hidden p-1",
                        if let Some(h) = heading {
                            div {
                                class: "text-muted-foreground px-3 py-2 text-xs font-medium",
                                "{h}"
                            }
                        }
                        for (_, item) in bucket {
                            {render_command_item_row(item, ctx)}
                        }
                    }
                }
            }
        }
    }
}

fn render_command_item_row(item: &CommandItemData, ctx: CommandContext) -> Element {
    let value = item.value.clone();
    let label = if item.label.is_empty() {
        item.value.clone()
    } else {
        item.label.clone()
    };
    let shortcut = item.shortcut.clone();
    let on_select = item.on_select;
    let disabled = item.disabled;

    rsx! {
        div {
            key: "{value}",
            class: crate::cn::merge(format!(
                "relative flex cursor-pointer items-center gap-2 rounded-xl px-3 py-2 text-sm hover:bg-muted hover:text-foreground transition-colors select-none [&_svg:not([class*='size-'])]:size-4 {}",
                if disabled { "opacity-50 pointer-events-none" } else { "" }
            )),
            onclick: move |_| {
                if disabled { return; }
                if let Some(cb) = &on_select {
                    cb.call(());
                }
                if let Some(close) = &ctx.on_close {
                    close.call(());
                }
            },
            span { class: "flex-1", "{label}" }
            if let Some(sc) = shortcut {
                span {
                    class: "ml-auto text-xs tracking-widest text-muted-foreground",
                    "{sc}"
                }
            }
        }
    }
}

// ── CommandGroup ────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct CommandGroupProps {
    /// Group heading text.
    pub heading: String,

    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// shadcn v4 maia: command group with heading
#[component]
pub fn CommandGroup(props: CommandGroupProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["overflow-hidden p-1", props.class.as_str()]),
            div {
                class: "text-muted-foreground px-3 py-2 text-xs font-medium",
                "{props.heading}"
            }
            {props.children}
        }
    }
}

// ── CommandItem ──────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct CommandItemProps {
    /// Search-matching value for this item. Fuzzy-matched against the
    /// `CommandInput` query via `nucleo-matcher`. Defaults to empty,
    /// which means the item shows for every query (use this for items
    /// you want pinned regardless of search).
    #[props(default)]
    pub value: String,

    /// Extra search terms — synonyms, abbreviations, emoji, etc. —
    /// matched alongside `value`. Mirrors shadcn cmdk's `keywords`.
    #[props(default)]
    pub keywords: Vec<String>,

    #[props(default)]
    pub on_select: Option<Callback<()>>,

    #[props(default)]
    pub class: String,

    pub children: Element,
}

/// shadcn v4 maia: single command item
#[component]
pub fn CommandItem(props: CommandItemProps) -> Element {
    let ctx: CommandContext = use_context();

    // Fuzzy-filter against the live search query — same matcher
    // (`nucleo-matcher`) that Combobox uses, so behaviour matches.
    // Items with an empty `value` and no `keywords` are always shown.
    let query = ctx.search_query.read().clone();
    if !query.is_empty() && !(props.value.is_empty() && props.keywords.is_empty()) {
        let mut candidates: Vec<&str> = vec![props.value.as_str()];
        candidates.extend(props.keywords.iter().map(|s| s.as_str()));
        if super::combobox::fuzzy_best_score(&query, &candidates).is_none() {
            return rsx! {};
        }
    }

    rsx! {
        div {
            class: crate::cn::merge(format!(
                "relative flex cursor-pointer items-center gap-2 rounded-xl px-3 py-2 text-sm hover:bg-muted hover:text-foreground transition-colors select-none [&_svg:not([class*='size-'])]:size-4 {}",
                props.class
            )),
            onclick: move |_| {
                if let Some(cb) = &props.on_select {
                    cb.call(());
                }
                if let Some(close) = &ctx.on_close {
                    close.call(());
                }
            },
            {props.children}
        }
    }
}

// ── CommandEmpty ─────────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct CommandEmptyProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: shown when command list is empty
#[component]
pub fn CommandEmpty(props: CommandEmptyProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["py-6 text-center text-sm text-muted-foreground", props.class.as_str()]),
            {props.children}
        }
    }
}

// ── CommandSeparator ────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct CommandSeparatorProps {
    #[props(default)]
    pub class: String,
}

/// shadcn v4 maia: command separator
#[component]
pub fn CommandSeparator(props: CommandSeparatorProps) -> Element {
    rsx! {
        div {
            class: crate::cn::merge_slice(&["bg-border/50 my-1 h-px", props.class.as_str()]),
            role: "separator",
        }
    }
}

// ── CommandShortcut ─────────────────────────────────────────────────────────

#[derive(Props, Clone, PartialEq)]
pub struct CommandShortcutProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: keyboard shortcut hint in command item
#[component]
pub fn CommandShortcut(props: CommandShortcutProps) -> Element {
    rsx! {
        span {
            class: crate::cn::merge_slice(&["ml-auto text-xs tracking-widest text-muted-foreground", props.class.as_str()]),
            {props.children}
        }
    }
}

/// Command palette using the data-driven `items` API: fuzzy-filtered,
/// sorted by score, grouped, with empty groups auto-hidden.
#[story(category = "Command", name = "command default")]
pub fn command_default() -> Element {
    let value = use_signal(String::new);
    let mut open = use_signal(|| true);

    let items = vec![
        CommandItemData::new("Calendar", "Calendar")
            .group("Suggestions")
            .keywords(["date", "schedule", "📅"]),
        CommandItemData::new("Search Emoji", "Search Emoji")
            .group("Suggestions")
            .keywords(["icon", "symbol", "😀"]),
        CommandItemData::new("Calculator", "Calculator")
            .group("Suggestions")
            .keywords(["math", "compute", "🧮"]),
        CommandItemData::new("Profile", "Profile")
            .group("Settings")
            .shortcut("⌘P")
            .keywords(["account", "user"]),
        CommandItemData::new("Billing", "Billing")
            .group("Settings")
            .shortcut("⌘B")
            .keywords(["payment", "invoice"]),
    ];

    rsx! {
        div { class: "p-6 bg-background text-foreground relative min-h-[24rem]",
            if !open() {
                button {
                    class: "h-9 px-3 rounded-lg border border-border text-sm",
                    onclick: move |_| open.set(true),
                    "Open command palette"
                }
            }
            CommandDialog {
                open: open(),
                on_close: move |_| open.set(false),
                CommandInput { value, placeholder: "Type a command...".to_string() }
                CommandList { items }
            }
        }
    }
}
