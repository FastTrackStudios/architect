//! The app in a browser (#172): the same shell and views as the desktop
//! and phone apps (`apps/ui`, on Blitz there), from the same Dioxus
//! components, here on dioxus-web.
//!
//! - the shell (`architect_shell::AppShell`): the bottom slots as
//!   favourites (`favourites`: Vault, Files, Search and New note, or the
//!   open note's type's own from the node's extensions, #607), the
//!   account, the foot's buttons, the status line, a phone's action bar;
//! - the Vault (`vault`): the workspace (`architect_workspace::WorkspaceView`:
//!   tabs, splits, docks, a phone's tab switcher), the navigator
//!   (`architect_navigator`) in its left dock, notes as their markdown
//!   (the editor's web view needs `eval`, which the page refuses: `page`);
//! - ⌘K and ⌘O: the commands (`architect_commands`) drawn by the Command
//!   Palette's views (`architect_command_palette::views`, without its
//!   extension: the browser runs no linked extension, #495);
//! - extensions' views in sandboxed frames of their own (`frames`, route A
//!   of #495): the view an island claims a note's type with, at the
//!   note's head (`embed`), and any view a favourite of its extension
//!   opens, in a pane, handed the note and its extension's playhead source;
//! - the Now Playing bar (`architect_playhead::ui::NowPlayingBar`, #589)
//!   in the shell's `now_playing` slot, over the deck every frame and the
//!   note views move, heard through the app's own audio element
//!   (`audio`);
//! - settings (`architect_settings::SettingsPage`): Appearance and the
//!   navigator's sections, kept in this browser (`localStorage`);
//! - the theme store (`architect_theme`), the system's light or dark.
//!
//! What a browser has no way to do yet is left out, not shown dead: pop-out
//! windows, a folder on this device, the files agent, linked extensions
//! (they need the Luau host, #495) and host calls from a frame into a vault
//! (#495's next step), capture's system-wide key, the menu bar.

// Nothing is `Send` in a browser.
#![allow(clippy::future_not_send)]

mod audio;
mod click;
mod embed;
mod favourites;
mod frames;
mod guest;
mod keys;
pub mod org;
mod page;
mod recording;
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
use lucide_dioxus::{Command as CommandIcon, Files, LogIn, LogOut, Search, Settings2, SquarePen};

/// Which app the page is.
#[derive(Clone)]
pub enum Mode {
    /// The demo vault in the tab's memory: nothing signed in, nothing sent.
    Demo,
    /// The signed-in account's org vaults, over the node (`org`).
    Org {
        /// The vaults, listed, and the lane they came over.
        vault: vault::WebVault,
        /// Who is signed in (their name, else their email).
        person: String,
        /// The org's slug.
        org: String,
    },
    /// A share link's guest (#629, #646): `/s/<code>/…`, the link's notes
    /// as its node hands them, no vault (`guest`).
    Guest {
        /// The link's code.
        code: String,
        /// The path below the link it was opened at (`/`, `/joy/`,
        /// `/_files/`).
        rest: String,
    },
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
    let web_vault = use_context_provider(|| match &mode {
        Mode::Demo => vault::WebVault {
            source: vault::demo_vault(),
            org: None,
        },
        Mode::Org { vault, .. } => vault.clone(),
        // No vault: everything a guest sees comes from the link's node.
        Mode::Guest { .. } => vault::WebVault {
            source: architect_navigator::MemorySource::new("Shared"),
            org: None,
        },
    });
    let workspace = vault::use_vault_workspace(&web_vault.source).signal();
    let screen = screen::use_browser_screen();
    let list = use_hook(|| Arc::new(ActionList::new()));
    use_context_provider(|| list.clone());
    use_hook(|| {
        let _ = architect::action::app_status::register(&list);
    });
    // The app's audio element, following the playhead (#589).
    use_hook(audio::start);
    use_hook(click::start);
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
    let demo = matches!(mode, Mode::Demo);
    let guest = matches!(mode, Mode::Guest { .. });
    // The card's art: an item's `art` (a vault path) as the demo vault's
    // image, a `data:` URL (`crate::demo::image_url`); a share link's, the
    // link's own file (`guest::file_url`).
    let art = use_callback(move |p: String| {
        if demo {
            crate::demo::image_url(&p)
        } else if guest {
            guest::file_url(&p)
        } else {
            None
        }
    });
    // The same for an extension's frames, so a frame and the card agree.
    use_context_provider(|| embed::ArtSource(art));
    // The demo's way out; signed in, the account's menu signs out.
    if demo {
        architect_commands::use_command(
            Command::new("WEB_SIGN_IN", "Sign in").group("App"),
            sign_in,
        );
    }
    // The Now Playing bar's presses on ⌘K, and a favourite's round button
    // (`PLAYHEAD_TOGGLE`, #607): the app's own, on the active source.
    for (id, name) in [
        ("PLAYHEAD_TOGGLE", "Play or pause"),
        ("PLAYHEAD_NEXT", "Next"),
        ("PLAYHEAD_PREVIOUS", "Previous"),
    ] {
        architect_commands::use_command(Command::new(id, name).group("Now Playing"), move || {
            architect_playhead::ui::press(id, &serde_json::json!({}));
        });
    }
    // The bottom slots: the open note's type's favourites, else the
    // web app's own.
    let slots = favourites::use_web_favourites(workspace, &web_vault.source, &commands);
    let set = slots.set.clone();
    let tabs = set.slots().to_vec();
    let separate = set.primary.clone();
    let current = slots.current;
    let foot = vec![shell::Action::new(
        "Settings",
        icon!(Settings2),
        move |()| {
            settings.show(None);
        },
    )];
    let account = match &mode {
        Mode::Demo => shell::Account::new("Demo")
            .detail("Nothing leaves this tab")
            .color("#b45309")
            .menu(
                shell::Menu::new()
                    .item("Sign in", icon!(LogIn), move |()| sign_in())
                    .item("Settings", icon!(Settings2), move |()| settings.show(None)),
            ),
        Mode::Org { person, org, .. } => shell::Account::new(person.clone())
            .detail(org.clone())
            .menu(
                shell::Menu::new()
                    .item("Settings", icon!(Settings2), move |()| settings.show(None))
                    .item("Sign out", icon!(LogOut), move |()| {
                        crate::browser::sign_out();
                    }),
            ),
        // Signed in or not, a link opens as its guest; "Sign in" goes to
        // the person's own vaults (`/app/`).
        Mode::Guest { .. } => shell::Account::new("Guest").detail("A shared link").menu(
            shell::Menu::new()
                .item("Sign in", icon!(LogIn), move |()| {
                    if let Some(w) = web_sys::window() {
                        let _ = w.location().assign(crate::APP);
                    }
                })
                .item("Settings", icon!(Settings2), move |()| settings.show(None)),
        ),
    };
    let mut status = vec![
        shell::StatusItem::new("demo", "Demo: nothing is saved")
            .hint("Sign in to keep your notes")
            .on_press(move |()| sign_in()),
        shell::StatusItem::new("commands", "⌘K Commands")
            .hint("Every command")
            .on_press(move |()| commands.show(Overlay::Palette)),
    ];
    // What failed on the org's vaults (a read, a new note, the session):
    // the latest, in the status line, until pressed.
    let mut problem = use_signal(|| None::<String>);
    use_hook(|| {
        if let Some(org) = &web_vault.org {
            let mut problems = org.problems();
            spawn(async move {
                use futures_util::StreamExt as _;
                while let Some(p) = problems.next().await {
                    problem.set(Some(p));
                }
            });
        }
    });
    match &mode {
        Mode::Demo => {}
        Mode::Guest { .. } => {
            status.retain(|s| s.id != "demo");
            status.insert(
                0,
                shell::StatusItem::new("guest", "Shared with you").hint("A share link, read-only"),
            );
        }
        Mode::Org { org, .. } => {
            status.retain(|s| s.id != "demo");
            status.insert(
                0,
                shell::StatusItem::new("org", format!("{org} · on the node")).hint(
                    "Your org's vaults: a note's text and new notes are written to the node; \
                     renaming, moving and deleting are in the desktop app",
                ),
            );
            if let Some(p) = problem() {
                status.insert(
                    1,
                    shell::StatusItem::new("problem", format!("⚠ {p}"))
                        .hint("Press to dismiss")
                        .on_press(move |()| problem.set(None)),
                );
            }
        }
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
                        current,
                        on_tab: move |id: &'static str| {
                            if !set.press(id) {
                                favourites::own_tab(id, workspace, &commands);
                            }
                        },
                        separate,
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
                        ]
                        .into_iter()
                        .chain(demo.then(|| shell::BarAction::new("WEB_SIGN_IN", "Sign in", icon!(LogIn))))
                        .collect::<Vec<_>>(),
                        now_playing: rsx! { architect_playhead::ui::NowPlayingBar { art } },
                        if let Mode::Guest { code, rest } = &mode {
                            guest::GuestPage { code: code.clone(), rest: rest.clone() }
                        } else {
                            vault::VaultPage {}
                        }
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
