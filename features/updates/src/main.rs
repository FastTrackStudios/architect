//! Updates as a process: the same `Updates` an app links in, served over
//! vox to the host that started it (`extension.toml`'s `[entry]`), as a
//! development copy does (`EXT_CLONE {"id":"updates"}`).

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_updates::Updates).serve()
}
