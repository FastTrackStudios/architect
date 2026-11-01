//! The in-window menu bar (feature `ui`): the [`MenuBar`] drawn by the app
//! itself, in the shell's theme, where the platform has no global one
//! (Windows, a Linux desktop that takes no dbusmenu: it stands aside when
//! [`Commands::has_global_menu`](crate::Commands::has_global_menu)). Each menu is
//! resolved when it opens, so its enabled states, ticks and live sections
//! are as they are then.

// The `rsx!` loop iterates menus collected so the tree is let go before
// drawing.
#![allow(clippy::needless_collect)]

use dioxus::prelude::*;
use lucide_dioxus::Check;

use crate::menu::{Kind, MenuBar, Pick, Tree, without_platform};
use architect_shell::{ShellTheme, try_use_shell};

use crate::ui::try_use_commands;

/// Draws `bar` as a row of menus (put it in the title row, or above the
/// page); picking a row runs its command.
#[component]
pub fn InWindowMenus(bar: MenuBar) -> Element {
    let Some(commands) = try_use_commands() else {
        return rsx! {};
    };
    if commands.has_global_menu() {
        return rsx! {};
    }
    let t = theme();
    let mut open = use_signal(|| None::<i32>);
    let tree = Tree::new(&without_platform(&bar.resolve(&commands.all())));
    let menus: Vec<(i32, String, Vec<Tree>)> = tree
        .children
        .into_iter()
        .filter_map(|m| match m.kind {
            Kind::Menu { title } => Some((m.id, title, m.children)),
            _ => None,
        })
        .collect();
    let showing = open();
    let pick = move |p: Pick| {
        open.set(None);
        let _ = commands.run_pick(&p);
    };
    rsx! {
        if showing.is_some() {
            // Anywhere else closes it.
            div {
                style: "position: absolute; top: 0; left: 0; width: 100vw; height: 100vh; z-index: 90;",
                onclick: move |_| open.set(None),
            }
        }
        div {
            role: "menubar",
            style: "display: flex; align-items: center; gap: 2px; height: 28px; font-family: {t.font}; font-size: 13px;",
            for (id, title, items) in menus {
                span { key: "{id}", style: "position: relative; z-index: 91;",
                    button {
                        class: "lp",
                        style: "height: 24px; padding: 0 8px; border: none; border-radius: 5px; color: {t.ink}; \
                                font-family: {t.font}; font-size: 13px; background: {fill(showing == Some(id), t.active)};",
                        onclick: move |_| open.set(if showing == Some(id) { None } else { Some(id) }),
                        // Once one is open, moving along opens the next.
                        onmouseenter: move |_| {
                            if showing.is_some() && showing != Some(id) {
                                open.set(Some(id));
                            }
                        },
                        "{title}"
                    }
                    if showing == Some(id) {
                        div {
                            role: "menu",
                            style: "position: absolute; top: 26px; left: 0; min-width: 220px; padding: 4px; \
                                    display: flex; flex-direction: column; border-radius: 8px; background: {t.pill}; \
                                    border: 1px solid {t.rule}; box-shadow: 0 12px 32px rgba(0,0,0,0.45);",
                            MenuRows { nodes: items, depth: 0, on_pick: pick }
                        }
                    }
                }
            }
        }
    }
}

fn theme() -> ShellTheme {
    try_use_shell().map(|s| s.theme).unwrap_or_default()
}

const fn fill(on: bool, active: &'static str) -> &'static str {
    if on { active } else { "transparent" }
}

/// A menu's rows; a menu inside it is its title, then its rows indented.
#[component]
fn MenuRows(nodes: Vec<Tree>, depth: u8, on_pick: EventHandler<Pick>) -> Element {
    let t = theme();
    let indent = 12_u16.saturating_mul(u16::from(depth)).saturating_add(8);
    rsx! {
        for node in nodes {
            match node.kind {
                Kind::Separator => rsx! {
                    div { key: "{node.id}", style: "height: 1px; margin: 4px 6px; background: {t.rule};" }
                },
                Kind::Menu { title } => rsx! {
                    div { key: "{node.id}", style: "padding: 4px {indent}px 2px; color: {t.ink_3}; font-size: 11px; font-weight: 600;",
                        "{title}"
                    }
                    MenuRows { nodes: node.children, depth: depth.saturating_add(1), on_pick }
                },
                Kind::Row(row) => {
                    let ink = if row.enabled { t.ink } else { t.ink_3 };
                    let enabled = row.enabled;
                    let pick = row.pick.clone();
                    rsx! {
                        div {
                            key: "{node.id}",
                            role: if row.checked.is_some() { "menuitemcheckbox" } else { "menuitem" },
                            "aria-disabled": !enabled,
                            "aria-checked": row.checked.map(|on| on.to_string()),
                            class: if enabled { "lp" } else { "" },
                            style: "display: flex; align-items: center; gap: 6px; height: 26px; padding: 0 10px 0 {indent}px; \
                                    border-radius: 5px; color: {ink};",
                            onclick: move |e| {
                                e.stop_propagation();
                                if enabled {
                                    on_pick.call(pick.clone());
                                }
                            },
                            span { style: "width: 14px; flex-shrink: 0; display: flex; align-items: center;",
                                if row.checked == Some(true) {
                                    Check { size: 13, color: ink }
                                }
                            }
                            span { style: "flex: 1; min-width: 0; white-space: nowrap; overflow: hidden;", "{row.label}" }
                            if !row.shortcut.is_empty() {
                                span { style: "color: {t.ink_3}; font-size: 12px;", "{row.shortcut}" }
                            }
                        }
                    }
                }
                Kind::Root | Kind::Standard(_) => rsx! {},
            }
        }
    }
}
