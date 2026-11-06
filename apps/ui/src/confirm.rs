//! The lab asks before an agent deletes: the Action List's confirmation
//! tickets, with the sheet a person answers them in
//! (`architect_commands::ConfirmSheet`).
//!
//! The lab's Delete is a command marked destructive, run by an agent
//! through `COMMANDS_RUN`, which reads each command's own effect: Delete
//! and the workspace's "Close other tabs" (and every destructive action)
//! wait for a yes, while every other command an agent runs goes ahead.
//! The desk lab can serve its list over MCP (`apps/desktop/src/mcp.rs`),
//! so a model can drive it and meet the sheet.

use architect::action::{ActionList, CONFIRM_TTL};

/// Turn the lab's confirmations on, on `list`.
pub fn require(list: &ActionList) {
    list.require_confirmation(CONFIRM_TTL);
}
