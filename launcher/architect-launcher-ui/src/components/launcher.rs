//! Launcher UI — composed from architect-ui primitives.

use architect_ui::prelude::*;
use dioxus::prelude::*;
use lucide_dioxus::{Search, Star, X};

use crate::state::LauncherState;
use architect_launcher_core::{ActionModifier, FilterState};

#[derive(Clone, Copy, PartialEq)]
enum SearchMode {
    Items,
    Filters,
}

#[component]
pub fn Launcher(state: Signal<LauncherState>, on_close: EventHandler<()>) -> Element {
    let mut query = use_signal(String::new);
    let mut selected_index = use_signal(|| 0usize);
    let mut filter = use_signal(FilterState::new);
    let mut search_mode = use_signal(|| SearchMode::Items);
    let mut preset_counter = use_signal(|| 0u32);
    let mut escape_armed = use_signal(|| false);

    let results = use_memo(move || {
        let q = query.read().clone();
        let f = filter.read().clone();
        state.read().query_filtered(&q, &f)
    });

    let result_count = results.read().len();
    if selected_index() >= result_count && result_count > 0 {
        selected_index.set(result_count - 1);
    }
    if result_count == 0 {
        selected_index.set(0);
    }

    // Pre-compute display rows to keep RSX flat (Blitz-friendly).
    let display_rows: Vec<DisplayRow> = results
        .read()
        .iter()
        .enumerate()
        .map(|(idx, item)| {
            let s = state.read();
            DisplayRow {
                idx,
                label: item.label.clone(),
                sub: item.sub.clone(),
                provider: item.provider.clone(),
                icon: item.icon.chars().next().unwrap_or('#').to_string(),
                tags: item
                    .tags
                    .tags()
                    .iter()
                    .take(3)
                    .map(|t| t.leaf().to_string())
                    .collect(),
                is_favorite: s.is_favorite(&item.id),
                rating: s.rating(&item.id),
                is_selected: idx == selected_index(),
            }
        })
        .collect();

    // Sidebar nodes (root tags).
    let sidebar_rows: Vec<SidebarRow> = {
        let s = state.read();
        let registry = s.engine().tag_registry();
        let active = filter.read().include.first().cloned();
        registry
            .root_tags()
            .iter()
            .map(|info| {
                let path = info.tag.path().to_string();
                SidebarRow {
                    is_active: active.as_deref() == Some(path.as_str()),
                    label: info.display_name.clone(),
                    path,
                }
            })
            .collect()
    };

    // Filter chips for the active filter set.
    let chip_rows: Vec<ChipRow> = filter
        .read()
        .include
        .iter()
        .map(|t| ChipRow {
            label: leaf(t),
            tag: t.clone(),
            excluded: false,
        })
        .chain(filter.read().exclude.iter().map(|t| ChipRow {
            label: leaf(t),
            tag: t.clone(),
            excluded: true,
        }))
        .collect();

    let preset_rows: Vec<String> = state
        .read()
        .presets()
        .iter()
        .map(|p| p.name.clone())
        .collect();

    let is_filter_mode = *search_mode.read() == SearchMode::Filters;
    let query_is_empty = query.read().is_empty() && filter.read().is_empty();
    let show_empty_search = result_count == 0 && !query_is_empty;
    let show_empty_idle = result_count == 0 && query_is_empty;

    // ── Handlers ───────────────────────────────────────────────────────────

    let activate_idx = move |idx: usize| {
        let r = results.read();
        if let Some(item) = r.get(idx) {
            if *search_mode.read() == SearchMode::Filters
                && let Some(first_tag) = item.tags.tags().first()
            {
                filter.write().toggle_include(first_tag.path());
                drop(r);
                query.set(String::new());
                selected_index.set(0);
                search_mode.set(SearchMode::Items);
                return;
            }
            let item = item.clone();
            let q = query.read().clone();
            let action_name = item
                .actions
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "activate".to_string());
            let should_close = state.read().activate(&item, &action_name, &q);
            if should_close {
                drop(r);
                on_close.call(());
            }
        }
    };

    let on_key = move |evt: KeyboardEvent| {
        let len = results.read().len();
        let m = evt.modifiers();
        let (ctrl, shift) = (m.ctrl(), m.shift());
        if !matches!(evt.key(), Key::Escape) {
            escape_armed.set(false);
        }
        if ctrl && matches!(evt.key(), Key::Character(ref c) if c == " ") {
            evt.prevent_default();
            on_close.call(());
            return;
        }
        match evt.key() {
            Key::ArrowDown => {
                evt.prevent_default();
                if len > 0 {
                    selected_index.set((selected_index() + 1) % len);
                }
            }
            Key::ArrowUp => {
                evt.prevent_default();
                if len > 0 {
                    selected_index.set(selected_index().checked_sub(1).unwrap_or(len - 1));
                }
            }
            Key::Tab => {
                evt.prevent_default();
                let next = match *search_mode.read() {
                    SearchMode::Items => SearchMode::Filters,
                    SearchMode::Filters => SearchMode::Items,
                };
                search_mode.set(next);
                selected_index.set(0);
            }
            Key::Enter => {
                evt.prevent_default();
                let r = results.read();
                if let Some(item) = r.get(selected_index()) {
                    if *search_mode.read() == SearchMode::Filters
                        && let Some(first_tag) = item.tags.tags().first()
                    {
                        filter.write().toggle_include(first_tag.path());
                        drop(r);
                        query.set(String::new());
                        selected_index.set(0);
                        search_mode.set(SearchMode::Items);
                        return;
                    }
                    let modifier = ActionModifier::from_modifiers(ctrl, shift, m.alt());
                    let action_name = item
                        .actions
                        .iter()
                        .find(|a| a.modifier == modifier)
                        .or_else(|| item.actions.first())
                        .map(|a| a.name.clone())
                        .unwrap_or_else(|| "activate".to_string());
                    let keep_open = item
                        .actions
                        .iter()
                        .find(|a| a.modifier == modifier)
                        .map(|a| a.keep_open)
                        .unwrap_or(false);
                    let item = item.clone();
                    let q = query.read().clone();
                    let should_close = state.read().activate(&item, &action_name, &q);
                    if should_close && !keep_open {
                        drop(r);
                        on_close.call(());
                    }
                }
            }
            Key::Escape => {
                evt.prevent_default();
                if escape_armed() {
                    on_close.call(());
                } else if !query.read().is_empty() {
                    query.set(String::new());
                    selected_index.set(0);
                    escape_armed.set(true);
                } else if *search_mode.read() == SearchMode::Filters {
                    search_mode.set(SearchMode::Items);
                    escape_armed.set(true);
                } else if !filter.read().is_empty() {
                    filter.write().clear();
                    selected_index.set(0);
                    escape_armed.set(true);
                } else {
                    on_close.call(());
                }
            }
            Key::Home => {
                evt.prevent_default();
                selected_index.set(0);
            }
            Key::End => {
                evt.prevent_default();
                if len > 0 {
                    selected_index.set(len - 1);
                }
            }
            Key::PageDown => {
                evt.prevent_default();
                if len > 0 {
                    selected_index.set((selected_index() + 10).min(len - 1));
                }
            }
            Key::PageUp => {
                evt.prevent_default();
                selected_index.set(selected_index().saturating_sub(10));
            }
            Key::Character(ref c) if c == ":" => {
                evt.prevent_default();
                on_close.call(());
            }
            Key::Character(ref c) if ctrl && c == "f" => {
                evt.prevent_default();
                if let Some(item) = results.read().get(selected_index()) {
                    state.read().toggle_favorite(&item.id);
                }
            }
            Key::Character(ref c) if ctrl && c == "l" => {
                evt.prevent_default();
                filter.write().clear();
                selected_index.set(0);
            }
            Key::Character(ref c) if ctrl && !shift && c == "s" => {
                evt.prevent_default();
                if !filter.read().is_empty() {
                    let f = filter.read().clone();
                    let n = preset_counter() + 1;
                    preset_counter.set(n);
                    state.read().save_preset(&format!("Preset {n}"), &f);
                }
            }
            Key::Character(ref c) if ctrl && shift && c.len() == 1 => {
                if let Some(d) = c.chars().next().and_then(|ch| ch.to_digit(10))
                    && (1..=5).contains(&d)
                {
                    evt.prevent_default();
                    if let Some(item) = results.read().get(selected_index()) {
                        let cur = state.read().rating(&item.id);
                        let new = if cur == d as u8 { 0 } else { d as u8 };
                        state.read().set_rating(&item.id, new);
                    }
                }
            }
            Key::Character(ref c) if ctrl && !shift && c.len() == 1 => {
                if let Some(d) = c.chars().next().and_then(|ch| ch.to_digit(10))
                    && (1..=9).contains(&d)
                {
                    let r = results.read();
                    if let Some(item) = r.get((d - 1) as usize) {
                        let item = item.clone();
                        let q = query.read().clone();
                        let should_close = state.read().activate(&item, "activate", &q);
                        if should_close {
                            drop(r);
                            on_close.call(());
                        }
                    }
                }
            }
            _ => {}
        }
    };

    rsx! {
        div {
            class: "flex h-screen w-full overflow-hidden bg-background text-foreground",
            onkeydown: on_key,

            // ── Sidebar ────────────────────────────────────────────────
            aside {
                class: "w-56 shrink-0 border-r border-border bg-sidebar text-sidebar-foreground flex flex-col",
                div { class: "px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground", "Tags" }
                div { class: "flex-1 overflow-y-auto px-2 pb-2 flex flex-col gap-0.5",
                    SidebarButton {
                        active: filter.read().include.is_empty(),
                        label: "All".to_string(),
                        on_click: move |_| {
                            filter.write().clear();
                            selected_index.set(0);
                        },
                    }
                    for row in sidebar_rows {
                        SidebarButton {
                            active: row.is_active,
                            label: row.label,
                            on_click: {
                                let path = row.path.clone();
                                move |_| {
                                    let mut f = filter.write();
                                    f.include.clear();
                                    f.exclude.clear();
                                    f.include.push(path.clone());
                                    drop(f);
                                    selected_index.set(0);
                                }
                            },
                        }
                    }
                }
            }

            // ── Main column ────────────────────────────────────────────
            div { class: "flex-1 min-w-0 flex flex-col overflow-hidden",

                // Search header
                div { class: "flex items-center gap-2 border-b border-border px-3 py-2",
                    Search { size: 16 }
                    input {
                        class: "flex-1 bg-transparent border-0 outline-none text-sm placeholder:text-muted-foreground",
                        r#type: "text",
                        placeholder: if is_filter_mode { "Search tags..." } else { "Search..." },
                        autofocus: true,
                        value: "{query}",
                        oninput: move |e| {
                            escape_armed.set(false);
                            query.set(e.value());
                            selected_index.set(0);
                        },
                    }
                    span { class: "text-xs text-muted-foreground tabular-nums",
                        "{result_count} results"
                    }
                }

                if is_filter_mode {
                    div { class: "px-3 py-1 text-[11px] uppercase tracking-wider text-primary bg-accent/40",
                        "Tag search — Tab to switch back · Enter applies"
                    }
                }

                // Filter chips
                if !chip_rows.is_empty() {
                    div { class: "flex flex-wrap items-center gap-1.5 border-b border-border px-3 py-2",
                        for chip in chip_rows {
                            FilterChip {
                                label: chip.label,
                                excluded: chip.excluded,
                                on_remove: {
                                    let tag = chip.tag.clone();
                                    move |_| {
                                        filter.write().remove_tag(&tag);
                                        selected_index.set(0);
                                    }
                                },
                            }
                        }
                        Button {
                            size: ButtonSize::Small,
                            variant: ButtonVariant::Ghost,
                            on_click: move |_| {
                                filter.write().clear();
                                selected_index.set(0);
                            },
                            "Clear"
                        }
                    }
                }

                // Preset bar
                if !preset_rows.is_empty() {
                    div { class: "flex flex-wrap items-center gap-1.5 border-b border-border px-3 py-2",
                        span { class: "text-[10px] uppercase tracking-wider text-muted-foreground mr-1",
                            "Presets"
                        }
                        for name in preset_rows {
                            Button {
                                size: ButtonSize::Small,
                                variant: ButtonVariant::Outline,
                                on_click: {
                                    let n = name.clone();
                                    move |_| {
                                        if let Some(fs) = state.read().load_preset(&n) {
                                            *filter.write() = fs;
                                            selected_index.set(0);
                                        }
                                    }
                                },
                                "{name}"
                            }
                        }
                    }
                }

                // Results
                div { class: "flex-1 overflow-y-auto px-2 py-1",
                    if show_empty_idle {
                        div { class: "flex flex-col items-center justify-center py-12 px-6 gap-2 text-muted-foreground",
                            div { class: "text-4xl opacity-40", "\u{2315}" }
                            div { class: "text-sm", "Start typing to search" }
                            div { class: "text-xs opacity-60",
                                "Tab: search modes · Ctrl+F: fav · Ctrl+Shift+1-5: rate · Ctrl+L: clear · Ctrl+S: save preset"
                            }
                        }
                    }
                    if show_empty_search {
                        div { class: "flex flex-col items-center justify-center py-12 px-6 gap-2 text-muted-foreground",
                            div { class: "text-4xl opacity-40", "\u{1F50D}" }
                            div { class: "text-sm", "No results" }
                        }
                    }

                    ItemGroup {
                        for row in display_rows {
                            ResultRow {
                                row,
                                on_click: activate_idx,
                            }
                        }
                    }
                }

                // Footer
                div { class: "flex items-center justify-between border-t border-border px-3 py-1.5 text-[11px] text-muted-foreground",
                    span {
                        if result_count > 0 { "{selected_index() + 1}/{result_count}" } else { "No results" }
                    }
                    span { class: "flex items-center gap-2",
                        Kbd { "↑↓" }
                        span { "navigate" }
                        Kbd { "Tab" }
                        span { "modes" }
                        Kbd { "Esc" }
                        span { "close" }
                        Kbd { ":" }
                        span { "close" }
                    }
                }
            }
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
struct DisplayRow {
    idx: usize,
    label: String,
    sub: String,
    provider: String,
    icon: String,
    tags: Vec<String>,
    is_favorite: bool,
    rating: u8,
    is_selected: bool,
}

#[derive(Clone, PartialEq)]
struct SidebarRow {
    label: String,
    path: String,
    is_active: bool,
}

#[derive(Clone, PartialEq)]
struct ChipRow {
    label: String,
    tag: String,
    excluded: bool,
}

fn leaf(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

#[component]
fn SidebarButton(active: bool, label: String, on_click: EventHandler<()>) -> Element {
    let cls = if active {
        "w-full text-left px-2 py-1.5 rounded-md text-sm bg-sidebar-accent text-sidebar-accent-foreground"
    } else {
        "w-full text-left px-2 py-1.5 rounded-md text-sm hover:bg-sidebar-accent/50"
    };
    rsx! {
        button {
            class: "{cls}",
            onclick: move |_| on_click.call(()),
            "{label}"
        }
    }
}

#[component]
fn FilterChip(label: String, excluded: bool, on_remove: EventHandler<()>) -> Element {
    let prefix = if excluded { "−" } else { "+" };
    let variant = if excluded {
        BadgeVariant::Destructive
    } else {
        BadgeVariant::Default
    };
    rsx! {
        Badge { variant,
            span { class: "flex items-center gap-1",
                span { "{prefix}{label}" }
                button {
                    class: "opacity-70 hover:opacity-100",
                    onclick: move |_| on_remove.call(()),
                    X { size: 12 }
                }
            }
        }
    }
}

#[component]
fn ResultRow(row: DisplayRow, on_click: EventHandler<usize>) -> Element {
    let variant = if row.is_selected {
        ItemVariant::Outline
    } else {
        ItemVariant::Default
    };
    let extra = if row.is_selected {
        "ring-1 ring-ring/50 bg-accent/40"
    } else {
        ""
    };
    let idx = row.idx;
    rsx! {
        div {
            onclick: move |_| on_click.call(idx),
            Item { variant, interactive: true, class: extra.to_string(),
                ItemMedia { "{row.icon}" }
                ItemContent {
                    div { class: "flex min-w-0 items-center gap-1.5 text-sm font-medium leading-none text-foreground",
                        span { class: "min-w-0 truncate", "{row.label}" }
                        if row.is_favorite { Star { size: 12 } }
                    }
                    if !row.sub.is_empty() {
                        div { class: "line-clamp-2 text-sm text-muted-foreground", "{row.sub}" }
                    }
                    if !row.tags.is_empty() {
                        div { class: "flex flex-wrap gap-1 mt-0.5",
                            for t in row.tags {
                                Badge { variant: BadgeVariant::Secondary, "{t}" }
                            }
                        }
                    }
                }
                ItemActions {
                    if row.rating > 0 {
                        span { class: "text-xs text-muted-foreground", "{row.rating}/5" }
                    }
                    Badge { variant: BadgeVariant::Outline, "{row.provider}" }
                }
            }
        }
    }
}
