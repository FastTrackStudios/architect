//! The lab asks before an agent deletes: the Action List's confirmation
//! tickets, with the sheet a person answers them in
//! (`architect_commands::ConfirmSheet`).
//!
//! The lab's Delete is a command, run by an agent through `COMMANDS_RUN`;
//! [`ActionList::set_confirm`] makes that one command (and closing other
//! tabs) wait for a yes, while every other command an agent runs goes
//! ahead. The desk lab can serve its list over MCP
//! (`apps/lab/desktop/src/mcp.rs`), so a model can drive it and meet the
//! sheet.

use architect::action::{ActionList, CONFIRM_TTL};

/// The commands an agent waits for a person's yes before running.
const ASKS: &[&str] = &["LAB_DELETE", "WORKSPACE_CLOSE_OTHER_TABS"];

/// Turn the lab's confirmations on, on `list`.
pub fn require(list: &ActionList) {
    list.require_confirmation(CONFIRM_TTL);
    list.set_confirm(architect_commands::RUN_ACTION, |args| {
        serde_json::from_slice::<serde_json::Value>(args)
            .ok()
            .and_then(|call| call.get("id")?.as_str().map(str::to_owned))
            .is_some_and(|id| ASKS.contains(&id.as_str()))
    });
}
