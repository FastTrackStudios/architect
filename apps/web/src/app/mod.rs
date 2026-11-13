//! The app in a browser (#172): the same shell and views as the desktop
//! and phone apps (`apps/ui`, on Blitz there), from the same Dioxus
//! components, here on dioxus-web.
//!
//! - the shell (`architect_shell::AppShell`): the Vault tab, the account,
//!   the foot's buttons, the status line, a phone's action bar;
//! - the Vault (`vault`): the workspace (`architect_workspace::WorkspaceView`:
//!   tabs, splits, docks, a phone's tab switcher), the navigator
//!   (`architect_navigator`) in its left dock, notes as their markdown
//!   (the editor's web view needs `eval`, which the page refuses: `page`);
//! - ⌘K and ⌘O: the commands (`architect_commands`) drawn by the Command
//!   Palette's views (`architect_command_palette::views`, without its
//!   extension: a browser has no extension host yet, #495);
//! - settings (`architect_settings::SettingsPage`): Appearance and the
//!   navigator's sections, kept in this browser (`localStorage`);
//! - the theme store (`architect_theme`), the system's light or dark.
//!
//! What a browser has no way to do yet is left out, not shown dead: pop-out
//! windows, a folder on this device, the files agent, extensions (linked
//! ones need the Luau host, #495), capture's system-wide key, the menu bar.

// Nothing is `Send` in a browser.
#![allow(clippy::future_not_send)]

mod keys;
mod page;
mod screen;
mod settings;
mod theme;
pub mod vault;

use std::rc::Rc;
use std::sync::Arc;

use architect::action::ActionList;
use architect_commands::{Command, Overlay, PaletteView};
use architect_shell::{self as shell, AppShell, icon};
use dioxus::prelude::*;
use lucide_dioxus::{
    Command as CommandIcon, Files, LogIn, NotebookText, Search, Settings2, SquarePen,
};

/// Which app the page is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The demo vault in the tab's memory: nothing signed in, nothing sent.
    Demo,
}

/// Mounts the app in `#app`, over what the page showed while it loaded.
pub fn mount(mode: Mode) {
    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
        if let Some(root) = doc.get_element_by_id("app") {
            root.set_inner_html("");
        }
        // The page is the app now: no site header, the whole window
        // (apps/web/app/style.css).
        if let Some(body) = doc.body() {
            let _ = body.set_attribute("data-mode", "app");
        }
    }
    let vdom = VirtualDom::new(WebApp).with_root_context(mode);
    dioxus::web::launch::launch_virtual_dom(vdom, dioxus::web::Config::new().rootname("app"));
}

/// Leaves the demo for the sign-in (the app's own URL, signed out).
fn sign_in() {
    let Some(w) = web_sys::window() else { return };
    // The demo keeps nothing: an edited one asks before it goes.
    if vault::demo_edited()
        && !w
            .confirm_with_message("Leave the demo? Your changes here are not saved.")
            .unwrap_or(false)
    {
        return;
    }
    let _ = w.location().assign(crate::APP);
}

/// The app's root: what `LabApp` (apps/ui) is on a desk or a phone, with
/// what a browser cannot do left out.
#[component]
fn WebApp() -> Element {
    let mode = use_context::<Mode>();
    // Before anything that evaluates: the page allows no `eval`.
    page::use_safe_document();
    page::use_keyboard_feed();
    let web_vault = use_context_provider(|| match mode {
        Mode::Demo => vault::WebVault {
            source: vault::demo_vault(),
        },
    });
    vault::use_vault_workspace(&web_vault.source);
    let screen = screen::use_browser_screen();
    let list = use_hook(|| Arc::new(ActionList::new()));
    use_context_provider(|| list.clone());
    use_hook(|| {
        let _ = architect::action::app_status::register(&list);
    });
    let theme = theme::use_web_theme(&list);
    let assets = use_hook(theme::assets);
    let settings = settings::use_web_settings();
    architect_settings::use_appearance_section();
    // Commands: the palette (⌘K), quick open (⌘O), every command's key.
    let commands = architect_commands::use_commands_provider();
    architect_commands::use_command_actions(list);
    // The Command Palette's views draw both overlays, as its extension
    // does on a desk (`[[palette_views]]`).
    use_hook(move || {
        commands.set_palette_views(vec![
            PaletteView {
                overlay: Overlay::Palette,
                owner: architect_command_palette::ID.to_owned(),
                draw: Rc::new(|| rsx! { architect_command_palette::views::PaletteView {} }),
            },
            PaletteView {
                overlay: Overlay::QuickOpen,
                owner: architect_command_palette::ID.to_owned(),
                draw: Rc::new(|| rsx! { architect_command_palette::views::QuickOpenView {} }),
            },
        ]);
    });
    architect_commands::use_command(
        Command::new(architect_command_palette::OPEN_ACTION, "Commands")
            .group("Help")
            .shortcut("⌘K"),
        move || commands.show(Overlay::Palette),
    );
    architect_commands::use_command(
        Command::new(architect_command_palette::OPEN_ANYTHING_ACTION, "Open…")
            .group("Go")
            .shortcut("⌘O"),
        move || commands.show(Overlay::QuickOpen),
    );
    architect_commands::use_command(
        Command::new("APP_SETTINGS", "Settings")
            .group("App")
            .shortcut("⌘,"),
        move || settings.show(None),
    );
    let demo = mode == Mode::Demo;
    architect_commands::use_command(Command::new("WEB_SIGN_IN", "Sign in").group("App"), sign_in);
    let tabs = vec![shell::Tab::new("vault", "Vault", icon!(NotebookText))];
    let foot = vec![shell::Action::new(
        "Settings",
        icon!(Settings2),
        move |()| {
            settings.show(None);
        },
    )];
    let menu = shell::Menu::new()
        .item("Sign in", icon!(LogIn), move |()| sign_in())
        .item("Settings", icon!(Settings2), move |()| settings.show(None));
    let account = shell::Account::new("Demo")
        .detail("Nothing leaves this tab")
        .color("#b45309")
        .menu(menu);
    let mut status = vec![
        shell::StatusItem::new("demo", "Demo: nothing is saved")
            .hint("Sign in to keep your notes")
            .on_press(move |()| sign_in()),
        shell::StatusItem::new("commands", "⌘K Commands")
            .hint("Every command")
            .on_press(move |()| commands.show(Overlay::Palette)),
    ];
    if !demo {
        status.retain(|s| s.id != "demo");
    }
    // A phone's demo: a strip over the top says so, the shell laid out
    // under it (as under a notch).
    let phone = screen().form() == shell::FormFactor::Phone;
    let strip = demo && phone;
    let shown = {
        let mut s = screen();
        if strip {
            s.insets.top += DEMO_STRIP;
        }
        s
    };
    let top = screen().insets.top;
    rsx! {
        div {
            style: "position: absolute; inset: 0; overflow: hidden;",
            document::Style { {theme::page_css(&theme.resolved().read(), assets.as_deref())} }
            theme::SystemScheme {}
            keys::WebKeys {}
            architect_ui::theme::ThemeStoreProvider {
                div { style: "position: absolute; inset: 0;",
                    AppShell {
                        screen: shown,
                        title: crate::browser::app_name(),
                        tabs,
                        current: Some("vault"),
                        on_tab: move |_: &'static str| {},
                        brand: rsx! { Brand {} },
                        account,
                        foot,
                        status,
                        // A phone's navigator: the shell's left side (the
                        // workspace's left dock is the shell's on a desk).
                        left: phone.then(|| rsx! { vault::FilesView {} }),
                        actions: vec![
                            shell::BarAction::new(vault::FILES, "Files", icon!(Files)),
                            shell::BarAction::new(architect_command_palette::OPEN_ANYTHING_ACTION, "Open…", icon!(Search)),
                            shell::BarAction::new(vault::NEW_NOTE, "New note", icon!(SquarePen)),
                            shell::BarAction::new(architect_command_palette::OPEN_ACTION, "Commands", icon!(CommandIcon)),
                            shell::BarAction::new("APP_SETTINGS", "Settings", icon!(Settings2)),
                            shell::BarAction::new("WEB_SIGN_IN", "Sign in", icon!(LogIn)),
                        ],
                        vault::VaultPage {}
                    }
                }
            }
            // A phone's demo strip over the top.
            if strip {
                div { style: "position: absolute; left: 0; right: 0; top: {top}px; height: {DEMO_STRIP}px; z-index: 30; \
                              display: flex; align-items: center; justify-content: center; gap: 8px; \
                              background: #b45309; color: #fff; font-size: 12px; font-weight: 600;",
                    span { "Demo: nothing is saved" }
                    DemoBadge { inverted: true }
                }
            }
            // On a desk the brand is not on show: the badge floats in the
            // title row's free end, beside its ⋯.
            if demo && !phone {
                div { style: "position: absolute; top: 8px; right: 52px; z-index: 30;",
                    DemoBadge {}
                }
            }
            architect_commands::CommandLayer {}
            architect_commands::ConfirmSheet {}
            architect_settings::SettingsPage {}
            SettingsLayer { phone }
        }
    }
}

/// The settings page over the app while a section is open: a card on a
/// desk, the whole screen on a phone (on the desktop app, the settings
/// extension's window). Esc, the sheet's close or a press outside close it.
#[component]
fn SettingsLayer(phone: bool) -> Element {
    let Some(h) = architect_settings::try_use_settings() else {
        return rsx! {};
    };
    if (h.open)().is_none() {
        return rsx! {};
    }
    let t = shell::ShellTheme::from_resolved(&architect_theme::use_resolved());
    let card = if phone {
        "position: absolute; inset: 0; border-radius: 0;"
    } else {
        "position: relative; width: min(960px, calc(100% - 48px)); height: min(700px, calc(100% - 48px)); border-radius: 14px;"
    };
    rsx! {
        div {
            "data-testid": "settings-layer",
            style: "position: absolute; inset: 0; z-index: 40; display: flex; align-items: center; justify-content: center; \
                    background: {t.scrim};",
            onclick: move |_| h.close(),
            onkeydown: move |e: KeyboardEvent| {
                if e.key() == Key::Named(NamedKey::Escape) {
                    h.close();
                }
            },
            div {
                style: "{card} overflow: hidden; display: flex; background: {t.desk}; color: {t.ink}; \
                        font-family: {t.font}; box-shadow: {t.shadow_lift};",
                onclick: move |e| e.stop_propagation(),
                architect_settings::SettingsSheet {}
            }
        }
    }
}

/// How tall a phone's demo strip is, points.
const DEMO_STRIP: f64 = 28.0;

/// The demo's mark: says it is the demo, and signs in (`inverted` on the
/// phone's amber strip).
#[component]
fn DemoBadge(#[props(default)] inverted: bool) -> Element {
    let (bg, fg, label) = if inverted {
        ("#fff", "#92400e", "Sign in")
    } else {
        ("#b45309", "#fff", "DEMO · Sign in")
    };
    rsx! {
        button {
            "data-testid": "demo-badge",
            title: "This is the demo: nothing is saved. Sign in to keep your notes.",
            style: "border: 0; border-radius: 999px; padding: 3px 10px; font: inherit; font-size: 11px; \
                    font-weight: 700; letter-spacing: 0.04em; background: {bg}; color: {fg}; cursor: pointer;",
            onclick: move |_| sign_in(),
            "{label}"
        }
    }
}

/// The app's name (a phone's top left).
#[component]
fn Brand() -> Element {
    let t = shell::use_shell().theme;
    rsx! {
        div { style: "display: flex; align-items: center; gap: 8px; font-weight: 600; color: {t.ink};",
            span { {crate::browser::app_name()} }

        }
    }
}
