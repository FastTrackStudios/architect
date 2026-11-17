//! The Command Palette's two views, linked in.
//!
//! Drawn in the shell's palette slot with the commands' kit
//! (`architect_commands::kit`): ⌘K's [`PaletteView`] and ⌘O's [`QuickOpenView`]. Linked in, they read the
//! app's `Commands`; a process copy draws [`crate::remote`]'s.

use std::rc::Rc;

use architect_commands::kit::{
    AskPalette, CommandRow, Frame, Lit, Pick, Reveal, fill, quiet, theme,
};
use architect_commands::{Found, OpenIn, Query, Ranked, Scope, try_use_commands};
use dioxus::prelude::*;

/// ⌘K: every command, matched as typed, and what the extensions' sources
/// find; ↑↓ to pick, ↵ to run. The `palette` view.
#[component]
pub fn PaletteView() -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let query = commands.palette_query();
    let mut selected = use_signal(|| 0usize);
    // Picked with the arrows (kept in sight), or under the pointer.
    let mut by_keys = use_signal(|| false);
    let q = Query::parse(&query.read());
    let only = commands.choice();
    let (rows, picks): (Vec<Ranked>, Vec<Pick>) = commands.palette_rows(&q).into_iter().unzip();
    let count = rows.len();
    // A new query starts at the top.
    use_effect(move || {
        let _ = query();
        selected.set(0);
    });
    if commands.is_asking() {
        return rsx! { AskPalette {} };
    }
    let enabled: Vec<bool> = rows.iter().map(|r| r.item.enabled).collect();
    // With nothing typed: what was run lately first, under its heading,
    // then the rest (`rank` puts them in that order).
    let headings: Vec<Option<&str>> = {
        let recents = commands.recents.read();
        let browsing =
            q.text.trim().is_empty() && only.is_none() && matches!(q.scope, Scope::Commands);
        let recent: Vec<bool> = rows
            .iter()
            .map(|r| recents.rank(&r.item.id).is_some())
            .collect();
        let any = browsing && recent.iter().any(|r| *r);
        let mut before = None::<bool>;
        recent
            .iter()
            .map(|&now| {
                let heading = match before {
                    _ if !any => None,
                    None => Some(if now { "Recent" } else { "All commands" }),
                    Some(true) if !now => Some("All commands"),
                    Some(_) => None,
                };
                before = Some(now);
                heading
            })
            .collect()
    };
    let pick = selected().min(count.saturating_sub(1));
    let run_at = move |i: usize| {
        if let (Some(p), Some(true)) = (picks.get(i), enabled.get(i)) {
            commands.pick(p);
        }
    };
    let run_key = run_at.clone();
    let on_key = move |e: KeyboardEvent| match e.key() {
        Key::Named(NamedKey::ArrowDown) => {
            e.prevent_default();
            by_keys.set(true);
            selected.set((pick.saturating_add(1)).min(count.saturating_sub(1)));
        }
        Key::Named(NamedKey::ArrowUp) => {
            e.prevent_default();
            by_keys.set(true);
            selected.set(pick.saturating_sub(1));
        }
        Key::Named(NamedKey::Enter) => run_key(pick),
        Key::Named(NamedKey::Escape) => commands.close(),
        _ => {}
    };
    let empty = match q.scope {
        Scope::Commands if q.text.is_empty() => "No commands yet",
        Scope::Commands => "No command matches",
        Scope::Targets => "Nothing on this page matches",
        Scope::Settings => "No setting matches",
        Scope::Help if q.text.is_empty() => "Press a key to see what it does",
        Scope::Help => "Nothing is bound to that",
    };
    rsx! {
        Frame { placeholder: only.clone().map_or_else(|| "Type a command…".to_owned(), |(_, prompt)| prompt), query, on_key,
            // The prompt stays in sight once the field has text.
            if let Some((_, prompt)) = only.filter(|_| !q.text.is_empty()) {
                div { style: "padding: 6px 10px 8px; color: {t.ink_3}; font-size: 12px;", "{prompt}" }
            }
            if rows.is_empty() {
                div { style: "padding: 18px 12px; color: {t.ink_3}; font-size: 13px;", "{empty}" }
            }
            for (i, row) in rows.into_iter().enumerate() {
                Fragment { key: "{row.item.source}:{row.item.id}",
                if let Some(heading) = headings.get(i).copied().flatten() {
                    div { style: "flex-shrink: 0; padding: 8px 10px 4px; color: {t.ink_3}; font-size: 11px; font-weight: 600;",
                        "{heading}"
                    }
                }
                Reveal { reveal: i == pick && by_keys(),
                CommandRow {
                    row,
                    on: i == pick,
                    onpress: {
                        let run_at = run_at.clone();
                        move |()| run_at(i)
                    },
                    onhover: move |()| {
                        by_keys.set(false);
                        selected.set(i);
                    },
                }
                }
                }
            }
        }
    }
}

/// ⌘O: open anything every opener knows; ↵ here, ⌘↵ in a new tab. The
/// `quick-open` view.
#[component]
pub fn QuickOpenView() -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    let t = theme();
    let mut query = use_signal(String::new);
    let mut selected = use_signal(|| 0usize);
    let mut by_keys = use_signal(|| false);
    // Asked from outside (`COMMANDS_SHOW`, an extension's picker): one
    // opener's items (`theme:`), the field empty for the person to narrow;
    // or the field set.
    let mut only = use_signal(|| None::<String>);
    use_effect(move || {
        let Some(q) = commands.take_seed() else {
            return;
        };
        let kind = q
            .strip_suffix(':')
            .map(str::trim)
            .filter(|k| commands.open_title(k).is_some());
        if let Some(k) = kind {
            only.set(Some(k.to_owned()));
            query.set(String::new());
        } else {
            only.set(None);
            query.set(q);
        }
    });
    // An opener that answers later (an extension's) bumps the version,
    // which `open_rows` reads.
    let asked = only().map_or_else(
        || query.read().clone(),
        |kind| format!("{kind}:{}", query.read()),
    );
    let found: Vec<Found> = commands.open_rows(&asked);
    let placeholder = only()
        .and_then(|k| commands.open_title(&k).map(|t| format!("{t}…")))
        .unwrap_or_else(|| "Open…  (note: to look in one place)".to_owned());
    let count = found.len();
    use_effect(move || {
        let _ = query();
        selected.set(0);
    });
    let pick = selected().min(count.saturating_sub(1));
    let items: Rc<Vec<Found>> = Rc::new(found);
    let open_at = {
        let items = items.clone();
        move |i: usize, open_in: OpenIn| {
            if let Some(f) = items.get(i) {
                commands.open_found(f, open_in);
            }
        }
    };
    let open_key = open_at.clone();
    let on_key = move |e: KeyboardEvent| match e.key() {
        Key::Named(NamedKey::ArrowDown) => {
            e.prevent_default();
            by_keys.set(true);
            selected.set((pick.saturating_add(1)).min(count.saturating_sub(1)));
        }
        Key::Named(NamedKey::ArrowUp) => {
            e.prevent_default();
            by_keys.set(true);
            selected.set(pick.saturating_sub(1));
        }
        Key::Named(NamedKey::Enter) => {
            let m = e.modifiers();
            let open_in = if m.meta() && m.alt() {
                OpenIn::Split
            } else if m.meta() || m.ctrl() {
                OpenIn::Tab
            } else {
                OpenIn::Here
            };
            open_key(pick, open_in);
        }
        Key::Named(NamedKey::Escape) => commands.close(),
        _ => {}
    };
    // Nothing typed: each kind's rows under its name, once; typed, they
    // mix by score and each row says its kind.
    let browsing = query.read().trim().is_empty();
    let titles: Vec<String> = items
        .iter()
        .map(|f| commands.open_title(&f.item.kind).unwrap_or_default())
        .collect();
    rsx! {
        Frame { placeholder, query, on_key,
            if items.is_empty() {
                div { style: "padding: 18px 12px; color: {t.ink_3}; font-size: 13px;", "Nothing to open matches" }
            }
            for (i, f) in items.iter().cloned().enumerate() {
                Fragment { key: "{f.item.kind}:{f.item.id}",
                if browsing && (i == 0 || titles.get(i) != titles.get(i.saturating_sub(1))) {
                    div { style: "flex-shrink: 0; padding: 8px 10px 4px; color: {t.ink_3}; font-size: 11px; font-weight: 600;",
                        "{titles.get(i).cloned().unwrap_or_default()}"
                    }
                }
                Reveal { reveal: i == pick && by_keys(),
                div {
                    class: "lp",
                    style: "flex-shrink: 0; min-height: 34px; box-sizing: border-box; display: flex; align-items: center; gap: 10px; \
                            padding: 5px 10px; border-radius: 7px; background: {fill(i == pick, &t)}; font-size: 13px; cursor: pointer;",
                    onclick: {
                        let open_at = open_at.clone();
                        move |_| open_at(i, OpenIn::Here)
                    },
                    onmouseenter: move |_| {
                        by_keys.set(false);
                        selected.set(i);
                    },
                    span { style: "flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 1px; overflow: hidden;",
                        span { style: "line-height: 18px;",
                            Lit { text: f.item.title.clone(), positions: f.positions.clone(), ink: t.ink.to_string(), lit: t.ink.to_string() }
                        }
                        if !f.item.detail.is_empty() {
                            span { style: "line-height: 15px; color: {quiet(i == pick, &t)}; font-size: 11px; white-space: nowrap; overflow: hidden;", "{f.item.detail}" }
                        }
                    }
                    if !browsing {
                        span { style: "flex-shrink: 0; color: {quiet(i == pick, &t)}; font-size: 11px;",
                            "{titles.get(i).cloned().unwrap_or_default()}"
                        }
                    }
                }
                }
                }
            }
            div { style: "display: flex; gap: 14px; padding: 8px 10px 4px; color: {t.ink_3}; font-size: 11px;",
                span { "Return  open" }
                span { "⌘ Return  new tab" }
                span { "⌘⌥ Return  split" }
            }
        }
    }
}
