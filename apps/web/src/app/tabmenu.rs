//! The menu a bottom tab opens above itself, on the signed-in bar (a
//! declared favourite whose target is a menu: a phone's "more").
//!
//! The tab's press fills [`MENU`] with its entries, each with what it
//! opens; [`TabMenu`], drawn once at the app's root, shows them over the
//! page, and a press outside closes it. A share link's guest page has its
//! own (`guest::TabMenu`), over the link's notes.

use dioxus::prelude::*;

/// One entry: its label, its Lucide icon by name, what it does.
#[derive(Clone, PartialEq)]
pub struct Entry {
    pub label: String,
    pub glyph: String,
    pub press: Callback<()>,
}

/// The open menu: its tab's label and its entries.
pub static MENU: GlobalSignal<Option<(String, Vec<Entry>)>> = Signal::global(|| None);

/// The open menu, over the page.
#[component]
pub fn TabMenu() -> Element {
    let t = architect_shell::use_shell().theme;
    let Some((label, entries)) = MENU() else {
        return rsx! {};
    };
    rsx! {
        div { "data-testid": "tab-menu-scrim", style: "position: fixed; inset: 0; z-index: 60;",
            onclick: move |_| *MENU.write() = None,
        }
        div { "data-testid": "tab-menu", role: "menu", "aria-label": "{label}",
            style: "position: fixed; z-index: 61; right: 76px; bottom: calc(env(safe-area-inset-bottom, 0px) + 76px); min-width: 208px; padding: 6px; border-radius: 14px; background: {t.sheet}; border: 1px solid {t.rule}; box-shadow: 0 2px 6px rgba(0,0,0,.12), 0 18px 40px -12px rgba(0,0,0,.45); display: flex; flex-direction: column; gap: 2px;",
            for (i, entry) in entries.into_iter().enumerate() {
                button { key: "{i}", r#type: "button", role: "menuitem", "data-testid": "tab-menu-item",
                    style: "display: flex; align-items: center; gap: 12px; width: 100%; min-height: 46px; padding: 0 12px; border: 0; border-radius: 10px; background: transparent; color: {t.ink}; font: inherit; font-size: 16px; text-align: left; cursor: pointer;",
                    onclick: move |_| entry.press.call(()),
                    span { style: "display: grid; place-items: center; width: 22px; height: 22px; color: {t.ink_3};",
                        {architect_shell::Tab::named("entry", "entry", super::guest::kept(&entry.glyph)).draw(architect_shell::Icon { size: 20, color: t.ink_3.clone() })}
                    }
                    "{entry.label}"
                }
            }
        }
    }
}
