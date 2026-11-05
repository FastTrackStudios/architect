//! A phone's Typing Toolbar as toolbars (feature `ui`, #316).
//!
//! The shell's bar over the keyboard ([`architect_shell::TypingTools`]),
//! filled with the toolbars of place `typing` whose `when` is in force
//! (`docs/content/architecture/app-shell.md`, "The action bar and the
//! keyboard bar").
//!
//! - The focused editor's own items (its `use_editing_actions`) are the
//!   built-in toolbar of its context, `typing/<context>`
//!   ([`crate::toolbars::Toolbars::provide_editor`]): a person's copy
//!   reorders, hides or adds to them, and they keep the editor's icons.
//! - An extension's, from its manifest or at run time, follow: shown
//!   while the page or view their `when` names has the focus.
//! - With the toolbar editor on, their strips show above the bar, edited
//!   in place as any toolbar is.
//!
//! The shell draws every item and keeps the field's focus through a
//! press; this side says what each is and runs a press as a toolbar's
//! (source `toolbar`, target `typing`, id the toolbar's key, the view the
//! focus is in).

use std::rc::Rc;

use architect::action::{Invocation, InvocationSource};
use architect_shell::{
    BarAction, Icon, IconFn, Typing, TypingButton, TypingContent, TypingItem, TypingMenu,
    TypingTools, TypingWidget, named_icon,
};
use dioxus::prelude::*;

use crate::interface::{ExtensionUi, ExtensionView};
use crate::toolbar::{MenuItem, MenuKind, TYPING, ToolItem, ToolKind};
use crate::toolbar_ui::ToolbarStrip;
use crate::toolbars::EDITOR;

/// Provides the shell's [`TypingTools`]: the toolbars of place `typing`
/// for what is typed in.
pub(crate) fn provide_typing_tools(ui: ExtensionUi) {
    use_context_provider(move || TypingTools::new(move |typing: &Typing| Some(fill(ui, typing))));
}

/// The Typing Toolbar's items for `typing`: each toolbar's in turn, a
/// separator between toolbars.
fn fill(ui: ExtensionUi, typing: &Typing) -> TypingContent {
    let (_, _) = (ui.revision(), ui.ticks());
    let host = ui.host();
    let bars = host.toolbars();
    // The editor's own items, kept as its context's built-in toolbar (a
    // change is heard as the toolbars', and drawn again).
    if let Some(editor) = &typing.editor {
        let own: Vec<(String, String)> = typing
            .actions
            .iter()
            .map(|a| (a.id.clone(), a.label.clone()))
            .collect();
        let _ = bars.provide_editor(editor, &own);
    }
    let shown = bars.typing(
        host.contributions(),
        typing.editor.as_deref(),
        &typing.contexts(),
    );
    let view = typing
        .scopes
        .iter()
        .rev()
        .find_map(|s| s.strip_prefix("view:"))
        .map(str::to_owned);
    let mut items = Vec::new();
    for t in &shown {
        let cx = Cx {
            ui,
            key: t.key.clone(),
            owner: t.owner.clone(),
            view: view.clone(),
        };
        let mut mine = Vec::new();
        for (n, item) in t.decl.items.iter().enumerate() {
            push(&cx, item, &n.to_string(), typing, &mut mine);
        }
        if !mine.is_empty() && !items.is_empty() {
            items.push(TypingItem::Separator);
        }
        items.extend(mine);
    }
    let above = (bars.editing() && !shown.is_empty()).then(|| {
        rsx! {
            for t in shown {
                ToolbarStrip { key: "{t.key}", ui, toolbar: t.key }
            }
        }
    });
    TypingContent { items, above }
}

/// What a toolbar's items are drawn and run with.
#[derive(Clone, PartialEq)]
struct Cx {
    ui: ExtensionUi,
    /// The toolbar's key and owner.
    key: String,
    owner: String,
    /// The view the focus is in (`lab.board/chart`).
    view: Option<String>,
}

impl Cx {
    fn invocation(&self) -> Invocation {
        let mut i = Invocation::new(InvocationSource::Toolbar).on(TYPING, self.key.clone());
        i.view.clone_from(&self.view);
        i
    }

    /// Runs `action` as pressed on this toolbar.
    fn run(&self, action: &str) {
        if let Some(c) = try_consume_context::<architect_commands::Commands>() {
            let _ = c.run_invoked(action, None, self.invocation());
        }
    }
}

/// No icon: a button drawn by its name.
fn no_icon(_: Icon) -> Element {
    VNode::empty()
}

/// `item`, at `path` in its toolbar, as the bar's items.
fn push(cx: &Cx, item: &ToolItem, path: &str, typing: &Typing, out: &mut Vec<TypingItem>) {
    match item.kind {
        ToolKind::Separator => out.push(TypingItem::Separator),
        // A group's buttons, one after another: the bar is one row.
        ToolKind::Group => {
            for (n, inner) in item.items.iter().enumerate() {
                push(cx, inner, &format!("{path}.{n}"), typing, out);
            }
        }
        ToolKind::Widget => {
            let (extension, view) = item.view.split_once('/').map_or_else(
                || (cx.owner.clone(), item.view.clone()),
                |(e, v)| (e.to_owned(), v.to_owned()),
            );
            let ui = cx.ui;
            out.push(TypingItem::Widget(TypingWidget {
                key: format!("{extension}/{view}"),
                width: f64::from(item.width.unwrap_or(120)),
                render: Rc::new(move || {
                    rsx! { ExtensionView { ui, extension: extension.clone(), view: view.clone(), fill: true } }
                }),
            }));
        }
        ToolKind::Button if item.action.is_empty() => {
            let icon = (!item.icon.is_empty()).then(|| named_icon(&item.icon));
            let label = if item.label.is_empty() {
                "More".to_owned()
            } else {
                item.label.clone()
            };
            let (run, rows) = (cx.clone(), item.menu.clone());
            out.push(TypingItem::Menu(TypingMenu {
                key: format!("{}#{path}", cx.key),
                label,
                icon,
                rows: Rc::new(move |close: Callback<()>| {
                    rsx! { TypingRows { cx: run.clone(), rows: rows.clone(), depth: 0, close } }
                }),
            }));
        }
        ToolKind::Button => out.push(button(cx, item, typing)),
    }
}

/// A button: its icon (its own, else the editor's for its action), its
/// name, a toggle's state from the app's list.
fn button(cx: &Cx, item: &ToolItem, typing: &Typing) -> TypingItem {
    let host = cx.ui.host();
    let info = host.list().info(&item.action);
    let builtin = typing.actions.iter().find(|a| a.id == item.action);
    let icon: Option<IconFn> = if item.icon.is_empty() {
        builtin.map(|a| a.icon)
    } else {
        Some(named_icon(&item.icon))
    };
    let label = [
        Some(item.label.clone()).filter(|l| !l.is_empty()),
        builtin.map(|a| a.label.clone()),
        info.as_ref().map(|i| i.display_name.clone()),
    ]
    .into_iter()
    .flatten()
    .next()
    .unwrap_or_else(|| item.action.clone());
    let mut action = BarAction::new(item.action.clone(), label, icon.unwrap_or(no_icon));
    if let Some(b) = builtin {
        action.badge.clone_from(&b.badge);
    }
    // An editor's own run as they always have, through the app's runner;
    // an extension's as pressed on its toolbar.
    let on_press = (cx.owner != EDITOR).then(|| {
        let (cx, id) = (cx.clone(), item.action.clone());
        Callback::new(move |()| cx.run(&id))
    });
    TypingItem::Button(TypingButton {
        action,
        checked: info.and_then(|i| i.checked),
        text: icon.is_none(),
        on_press,
    })
}

/// A dropdown's rows, above the bar: actions (ticked while a toggle is
/// on), headings, separators, and submenus that open in place. A row
/// pressed runs and closes it; the shell keeps the field's focus.
#[component]
fn TypingRows(cx: Cx, rows: Vec<MenuItem>, depth: usize, close: Callback<()>) -> Element {
    let (_, _) = (cx.ui.revision(), cx.ui.ticks());
    let t = crate::window_ui::shell_theme();
    let mut expanded = use_signal(|| None::<usize>);
    let host = cx.ui.host();
    let indent = depth.saturating_mul(12);
    rsx! {
        for (n, row) in rows.into_iter().enumerate() {
            match row.kind {
                MenuKind::Heading => rsx! {
                    div { key: "{n}", style: "padding:8px 10px 2px {10 + indent}px; font-size:12px; font-weight:600; color:{t.ink_3};", "{row.label}" }
                },
                MenuKind::Separator => rsx! {
                    div { key: "{n}", role: "separator", style: "height:1px; margin:4px 8px; background:{t.rule};" }
                },
                MenuKind::Submenu => {
                    let on = expanded() == Some(n);
                    rsx! {
                        button { key: "{n}",
                            tabindex: "-1",
                            role: "menuitem",
                            "aria-expanded": "{on}",
                            style: "display:flex; justify-content:space-between; align-items:center; padding:0 10px 0 {10 + indent}px; \
                                    min-height:40px; border:none; border-radius:8px; background:transparent; color:{t.ink}; font-size:15px;",
                            onclick: move |_| expanded.set((!on).then_some(n)),
                            span { "{row.label}" }
                            span { if on { "▾" } else { "›" } }
                        }
                        if on {
                            TypingRows { key: "{n}-in", cx: cx.clone(), rows: row.items, depth: depth.saturating_add(1), close }
                        }
                    }
                }
                MenuKind::Action => {
                    let info = host.list().info(&row.action);
                    let checked = info.as_ref().and_then(|i| i.checked);
                    let label = if row.label.is_empty() {
                        info.map_or_else(|| row.action.clone(), |i| i.display_name)
                    } else {
                        row.label.clone()
                    };
                    let tick = if checked == Some(true) { "✓" } else { "" };
                    let (run, action) = (cx.clone(), row.action.clone());
                    rsx! {
                        button { key: "{n}",
                            tabindex: "-1",
                            role: if checked.is_some() { "menuitemcheckbox" } else { "menuitem" },
                            "aria-checked": checked.map(|on| on.to_string()),
                            "data-tool": "{row.action}",
                            style: "display:flex; align-items:center; gap:8px; padding:0 10px 0 {indent}px; min-height:40px; \
                                    border:none; border-radius:8px; background:transparent; color:{t.ink}; font-size:15px; text-align:left;",
                            onclick: move |e: MouseEvent| {
                                e.prevent_default();
                                close.call(());
                                run.run(&action);
                            },
                            span { style: "flex:0 0 auto; width:18px; text-align:center; color:{t.accent};", "{tick}" }
                            span { "{label}" }
                        }
                    }
                }
            }
        }
    }
}
