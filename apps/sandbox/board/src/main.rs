//! The Board extension as a process: the same `Board` the lab links in,
//! served over vox to the host that started it (`extension.toml`'s
//! `[entry]`). Run by hand it says so and exits.

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_sandbox_board::Board).serve()
}
