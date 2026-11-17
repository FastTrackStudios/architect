//! A desk's sidebar layouts on the palette: "Sidebar: Rail", "Sidebar: One
//! column", "Sidebar: Two columns" ([`architect_shell::SidebarLayout`]),
//! every one on every page, the one on show disabled; and "Toggle the
//! sidebar" ([`TOGGLE_SIDEBAR`]): a phone's left drawer opened or closed,
//! a desk's sidebar folded or unfolded.

use std::sync::Arc;

use architect::action::ActionList;
use architect_shell::{Chrome, Shell, Side, SidebarLayout, Slot};
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

/// "Toggle the rail": the desk's ribbon shown or hidden (its slot,
/// `HOME_SLOT_SET`).
pub const TOGGLE_RAIL: &str = "SHELL_RAIL_TOGGLE";
/// "Toggle the status bar": a desk's status bar shown or hidden.
pub const TOGGLE_STATUS: &str = "SHELL_STATUS_TOGGLE";
/// "Toggle the right sidebar": the right dock opened or folded to its
/// rail (the workspace's command).
pub const TOGGLE_RIGHT: &str = "WORKSPACE_TOGGLE_RIGHT_DOCK";

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
    // The rail and the status bar: the shell's slots, shown or hidden in
    // the layout on show by hand (`HOME_SLOT_SET`, kept apart from the
    // layout's file).
    let chrome = try_use_context::<Signal<Chrome>>();
    let list = use_hook(try_consume_context::<Arc<ActionList>>);
    for (id, label, slot, name) in [
        (TOGGLE_RAIL, "Toggle the rail", Slot::Ribbon, "ribbon"),
        (
            TOGGLE_STATUS,
            "Toggle the status bar",
            Slot::Status,
            "status",
        ),
    ] {
        let command = Command::new(id, label).group("View");
        let command = if desk && list.is_some() {
            command
        } else {
            command.disabled("a desk's")
        };
        let list = list.clone();
        use_command(command, move || {
            let hidden = chrome.is_some_and(|c| c.peek().is_hidden(slot));
            if let Some(list) = &list {
                let args = serde_json::json!({ "slot": name, "hidden": !hidden }).to_string();
                // An app with no layouts (no `HOME_*`) has no slot to set.
                let _ = list.run_json("HOME_SLOT_SET", args.as_bytes());
            }
        });
    }
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
