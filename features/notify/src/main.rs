//! Notify as a process: the same `Notify` an app links in, served over
//! vox to the host that started it (`extension.toml`'s `[entry]`), as a
//! development copy does (`EXT_CLONE {"id":"notify"}`).

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_notify::Notify).serve()
}
