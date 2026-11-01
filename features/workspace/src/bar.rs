//! The workspace's button on a phone's action bar (feature `ui`): the
//! tabs, counted, opening the tab switcher.

use architect_shell::{BarAction, icon, use_bar_action};
use dioxus::prelude::*;
use lucide_dioxus::Layers;

use crate::model::Workspace;

/// The command the tabs button runs: the host's, which on a phone opens
/// the tab switcher.
pub const TABS_COMMAND: &str = "WORKSPACE_EVERY_PANE";

/// Puts the tabs button on a phone's action bar while the calling
/// component is mounted.
///
/// `workspace`'s tabs are counted on it (its main window's,
/// [`Workspace::tab_count`]); a press opens the tab switcher
/// ([`TABS_COMMAND`]). Mount it beside the
/// [`WorkspaceView`](crate::WorkspaceView) that draws `workspace`.
pub fn use_tabs_button(workspace: Signal<Workspace>) {
    let open = workspace.read().tab_count();
    use_bar_action(BarAction::new(TABS_COMMAND, "Tabs", icon!(Layers)).badge(open));
}
