//! Threads as a process: the same `Threads` an app links in, served over
//! vox to the host that started it (`extension.toml`'s `[entry]`), as a
//! development copy does (`EXT_CLONE {"id":"threads"}`).

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_threads::Threads).serve()
}
