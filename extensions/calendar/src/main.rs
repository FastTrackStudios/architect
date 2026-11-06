//! The Calendar as a process: the same `Calendar` the app links in,
//! served over vox to the host that started it (`extension.toml`'s
//! `[entry]`). Run by hand it says so and exits.

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_calendar::Calendar).serve()
}
