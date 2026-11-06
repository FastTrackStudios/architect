//! The Command Palette as a process: the same `CommandPalette` an app
//! links in, served over vox to the host that started it
//! (`extension.toml`'s `[entry]`), as a development copy does
//! (EXT_CLONE {"id":"command-palette"}). Run by hand it says so and exits.

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_command_palette::CommandPalette).serve()
}
