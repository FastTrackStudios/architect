//! The menu a bottom tab opens above itself: a declared favourite whose
//! target is a menu (a setlist's More: Arrangement, Mixer, Lighting).
//!
//! The tab's press fills [`TAB_MENU`] with its entries, each with what it
//! does; the phone's foot draws them above the bar ([`TabMenu`]), and a
//! press outside, or on the tab again, closes it. The same in every app
//! the shell draws (the web app and the native one alike).

use dioxus::prelude::*;

use super::{Icon, Tab, intern, use_shell};

/// One entry: its label, its Lucide icon by name, what it does.
#[derive(Clone, PartialEq)]
pub struct TabMenuEntry {
    pub label: String,
    pub glyph: String,
    pub press: Callback<()>,
}

/// The open menu: its tab's label and its entries.
pub static TAB_MENU: GlobalSignal<Option<(String, Vec<TabMenuEntry>)>> = Signal::global(|| None);

/// Opens `entries` as tab `label`'s menu, or closes the menu when one is
/// open (the tab pressed again).
pub fn toggle_tab_menu(label: &str, entries: Vec<TabMenuEntry>) {
    if TAB_MENU.peek().is_some() {
        *TAB_MENU.write() = None;
    } else {
        *TAB_MENU.write() = Some((label.to_owned(), entries));
    }
}

/// Closes the open menu, if one is.
pub fn close_tab_menu() {
    if TAB_MENU.peek().is_some() {
        *TAB_MENU.write() = None;
    }
}

/// The open menu, above the bar it is drawn over (its parent is the
/// bar's box, positioned): a scrim over the page that closes it, and the
/// entries at the bar's right, over the round button's column.
#[component]
pub(super) fn TabMenu() -> Element {
    let t = use_shell().theme;
    let Some((label, entries)) = TAB_MENU() else {
        return rsx! {};
    };
    rsx! {
        div { "data-testid": "tab-menu-scrim",
            style: "position: absolute; left: 0; right: 0; bottom: 0; height: 4000px; z-index: 60;",
            onclick: move |_| close_tab_menu(),
        }
        div { "data-testid": "tab-menu", role: "menu", "aria-label": "{label}",
            style: "position: absolute; z-index: 61; right: 76px; bottom: calc(100% + 8px); min-width: 208px; padding: 6px; box-sizing: border-box; border-radius: 14px; background: {t.sheet}; border: 1px solid {t.rule}; box-shadow: 0 18px 40px -12px rgba(0,0,0,.45); display: flex; flex-direction: column; gap: 2px;",
            for (i, entry) in entries.into_iter().enumerate() {
                button { key: "{i}", r#type: "button", role: "menuitem", "data-testid": "tab-menu-item",
                    style: "display: flex; align-items: center; gap: 12px; width: 100%; min-height: 46px; padding: 0 12px; border: 0; border-radius: 10px; background: transparent; color: {t.ink}; font: inherit; font-size: 16px; text-align: left; cursor: pointer;",
                    onclick: move |_| {
                        close_tab_menu();
                        entry.press.call(());
                    },
                    span { style: "display: flex; align-items: center; justify-content: center; width: 22px; height: 22px;",
                        {Tab::named("entry", "entry", intern(&entry.glyph)).draw(Icon { size: 20, color: t.ink_3.clone().into() })}
                    }
                    "{entry.label}"
                }
            }
        }
    }
}
