//! A desk's sidebar layouts on the palette: "Sidebar: Rail", "Sidebar: One
//! column", "Sidebar: Two columns" ([`architect_shell::SidebarLayout`]),
//! every one on every page, the one on show disabled.

use architect_shell::{Shell, SidebarLayout};
use dioxus::prelude::*;

use crate::{Command, use_command};

/// The command id for `layout`: `SHELL_SIDEBAR_RAIL`, `SHELL_SIDEBAR_ONE`,
/// `SHELL_SIDEBAR_TWO`.
#[must_use]
pub const fn sidebar_command(layout: SidebarLayout) -> &'static str {
    match layout {
        SidebarLayout::Rail => "SHELL_SIDEBAR_RAIL",
        SidebarLayout::One => "SHELL_SIDEBAR_ONE",
        SidebarLayout::Two => "SHELL_SIDEBAR_TWO",
    }
}

/// Puts the three sidebar layouts on the palette. Call it once, inside
/// the app's shell (under its `AppShell`), on a desk or not: off a desk
/// they are disabled.
pub fn use_sidebar_commands() {
    let shell = try_use_context::<Signal<Shell>>();
    let (now, desk) = shell.map_or((None, false), |s| {
        let s = s.read();
        (Some(s.sidebar_layout()), s.desk())
    });
    for layout in SidebarLayout::ALL {
        let command = Command::new(
            sidebar_command(layout),
            format!("Sidebar: {}", layout.label()),
        )
        .group("View");
        let command = if !desk {
            command.disabled("a desk's sidebar")
        } else if now == Some(layout) {
            command.disabled("it is the sidebar's layout now")
        } else {
            command
        };
        use_command(command, move || {
            if let Some(shell) = shell {
                shell.peek().set_sidebar_layout(layout);
            }
        });
    }
}
