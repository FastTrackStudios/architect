//! The workspace's button on a phone's action bar (feature `ui`): the
//! tabs, counted, opening the pane switcher.

use architect_shell::{BarAction, icon, use_bar_action};
use dioxus::prelude::*;
use lucide_dioxus::Layers;

use crate::model::Workspace;

/// The command the tabs button runs: the host's, which on a phone opens
/// the switcher of every pane.
pub const TABS_COMMAND: &str = "WORKSPACE_EVERY_PANE";

/// Puts the tabs button on a phone's action bar while the calling
/// component is mounted.
///
/// `workspace`'s open panes are counted on it; a press opens the switcher
/// ([`TABS_COMMAND`]). Mount it beside the
/// [`WorkspaceView`](crate::WorkspaceView) that draws `workspace`.
pub fn use_tabs_button(workspace: Signal<Workspace>) {
    let open = workspace.read().panes.len();
    use_bar_action(BarAction::new(TABS_COMMAND, "Tabs", icon!(Layers)).badge(open));
}
