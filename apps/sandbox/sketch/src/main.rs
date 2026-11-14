//! Sketch's process: its three views.

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_sandbox_sketch::Sketch).serve()
}
