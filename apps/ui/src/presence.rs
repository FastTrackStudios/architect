//! The app's presence on a desk.
//!
//! The Inbox's count on the Dock icon, a Dock menu and a menu-bar item
//! from the lab's commands, background mode (closing the window leaves
//! the lab in the menu bar) and opening at login. A phone has none of it.

use architect_commands::{Live, Menu};
use architect_presence::{
    Presence, QUIT, SHOW_WINDOW, TOGGLE_BACKGROUND, TOGGLE_LOGIN, Tray, TrayIcon,
};
use dioxus::prelude::*;

use crate::{Lab, tabs};

/// The running app's bundle identifier (its distribution's): its one
/// instance, its login item.
#[must_use]
pub fn id() -> &'static str {
    crate::distribution().id()
}

/// What the lab declares: its menus read the tab on show, live. With
/// `capture` (the Architect app's, #352), the menu-bar item opens the
/// capture window, over whatever app is in front.
#[must_use]
pub fn declare(lab: Lab, inbox: usize, capture: bool) -> Presence {
    let tabs = move || {
        let on = (lab.route)().tab();
        tabs()
            .iter()
            .map(|(id, label, to)| Live::new(*label, *id).checked(*to == on))
            .collect()
    };
    let dock = Menu::new("Dock")
        .command("LAB_CAPTURE")
        .separator()
        .section("Tabs", tabs)
        .separator()
        .command(TOGGLE_BACKGROUND);
    let dist = crate::distribution();
    let tray = Menu::new(dist.name())
        .command(SHOW_WINDOW)
        .command(if capture {
            crate::capture::WINDOW_COMMAND
        } else {
            "LAB_CAPTURE"
        });
    let tray = tray
        .separator()
        .section("Tabs", tabs)
        .separator()
        .commands([TOGGLE_BACKGROUND, TOGGLE_LOGIN])
        .separator()
        .command(QUIT);
    // The distribution's marks (`architect_distribution::Icon`).
    let icons = dist.icons();
    let icon = TrayIcon::symbol(icons.tray_symbol).icon_name(icons.tray_name);
    Presence::new(id(), dist.name())
        .dock_menu(dock)
        .tray(
            Tray::new(icon, tray)
                .tooltip(dist.name())
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
                // A second launch's URL, as a tab to go to
                // (`architect://vault`; the lab's `lab://` too).
                on_handoff: move |h: architect_presence::Handoff| {
                    for url in &h.urls {
                        // A share link (or `architect://open?url=…`):
                        // opened as a guest, as the OS's (#629).
                        if architect_spaces::ShareUrl::parse(url).is_ok() {
                            crate::guest::incoming(url.clone(), crate::guest::Source::Os);
                            continue;
                        }
                        let tab = url
                            .strip_prefix("architect://")
                            .or_else(|| url.strip_prefix("lab://"))
                            .unwrap_or(url);
                        if let Some((_, _, to)) = tabs().iter().find(|(_, _, t)| *t == tab) {
                            lab.jump(crate::Route::Tab(to));
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
