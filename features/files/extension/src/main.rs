//! Files as a process: the same `FilesExtension` the app links in, served
//! over vox to the host that started it (`extension.toml`'s `[entry]`).

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_files_extension::FilesExtension).serve()
}
