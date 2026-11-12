//! A desk's sidebar layouts on the palette: "Sidebar: Rail", "Sidebar: One
//! column", "Sidebar: Two columns" ([`architect_shell::SidebarLayout`]),
//! every one on every page, the one on show disabled; and "Toggle the
//! sidebar" ([`TOGGLE_SIDEBAR`]): a phone's left drawer opened or closed,
//! a desk's sidebar folded or unfolded.

use architect_shell::{Shell, Side, SidebarLayout};
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

/// "Toggle the sidebar": the left side opened or closed, on every form
/// (a phone's drawer, a tablet's or desk's folding sidebar).
pub const TOGGLE_SIDEBAR: &str = "SHELL_SIDEBAR_TOGGLE";

/// Puts the three sidebar layouts on the palette. Call it once, inside
/// the app's shell (under its `AppShell`), on a desk or not: off a desk
/// they are disabled.
pub fn use_sidebar_commands() {
    let shell = try_use_context::<Signal<Shell>>();
    let (now, desk) = shell.map_or((None, false), |s| {
        let s = s.read();
        (Some(s.sidebar_layout()), s.desk())
    });
    let has_left = shell.is_some_and(|s| s.read().has_side(Side::Left));
    let toggle = Command::new(TOGGLE_SIDEBAR, "Toggle the sidebar").group("View");
    use_command(
        if has_left {
            toggle
        } else {
            toggle.disabled("this page has no sidebar")
        },
        move || {
            if let Some(shell) = shell {
                shell.peek().toggle(Side::Left);
            }
        },
    );
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
