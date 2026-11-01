//! The lab's presence on a desk.
//!
//! The Inbox's count on the Dock icon, a Dock menu and a menu-bar item
//! from the lab's commands, background mode (closing the window leaves
//! the lab in the menu bar) and opening at login. A phone has none of it.

use architect_commands::{Live, Menu};
use architect_presence::{
    Presence, QUIT, SHOW_WINDOW, TOGGLE_BACKGROUND, TOGGLE_LOGIN, Tray, TrayIcon,
};
use dioxus::prelude::*;

use crate::{Lab, Route, TABS};

/// The desktop lab's bundle identifier.
pub const ID: &str = "app.fasttrackstudio.architect-desktop";

/// What the lab declares: its menus read the tab on show, live.
#[must_use]
pub fn declare(lab: Lab, inbox: usize) -> Presence {
    let tabs = move || {
        let on = (lab.route)().tab();
        TABS.iter()
            .map(|(id, label, to)| Live::new(*label, *id).checked(*to == on))
            .collect()
    };
    let dock = Menu::new("Dock")
        .command("LAB_CAPTURE")
        .separator()
        .section("Tabs", tabs)
        .separator()
        .command(TOGGLE_BACKGROUND);
    let tray = Menu::new("Architect Lab")
        .command(SHOW_WINDOW)
        .command("LAB_CAPTURE")
        .separator()
        .section("Tabs", tabs)
        .separator()
        .commands([TOGGLE_BACKGROUND, TOGGLE_LOGIN])
        .separator()
        .command(QUIT);
    Presence::new(ID, "Architect Lab")
        .dock_menu(dock)
        .tray(
            Tray::new(
                TrayIcon::symbol("testtube.2").icon_name("applications-science"),
                tray,
            )
            .tooltip("Architect Lab")
            .title((inbox > 0).then(|| inbox.to_string())),
        )
        .background(true)
        .open_at_login()
}

/// The presence, bound to the lab's state (a desk only).
#[component]
pub fn LabPresence(presence: Presence, inbox: usize) -> Element {
    #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
    {
        let lab = use_context::<Lab>();
        rsx! {
            architect_presence::AppPresence {
                presence,
                badge: architect_presence::Badge::count(inbox),
                // A second launch's URL, as a tab to go to (`lab://vault`).
                on_handoff: move |h: architect_presence::Handoff| {
                    for url in &h.urls {
                        let tab = url.trim_start_matches("lab://");
                        if let Some((_, _, to)) = TABS.iter().find(|(_, _, t)| *t == tab) {
                            lab.jump(Route::Tab(to));
                        }
                    }
                },
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (presence, inbox);
        rsx! {}
    }
}
