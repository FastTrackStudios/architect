//! Kanban-lite's process: its board view and its one action.

use architect_extensions::guest::GuestError;

fn main() -> Result<(), GuestError> {
    architect_sandbox_kanban::guest().serve()
}
