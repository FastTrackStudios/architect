//! The theme the extensions' chrome draws with (feature `ui`), read from
//! context: the shell's, else the app's theme store's.

use architect_shell::{ShellTheme, try_use_shell};

/// The shell's theme where there is a shell in context, else the app's
/// theme store's, as a shell theme. Read in a render: the component
/// renders again when it changes.
#[must_use]
pub fn shell_theme() -> ShellTheme {
    try_use_shell().map_or_else(
        || ShellTheme::from_resolved(&architect_theme::use_resolved()),
        |s| s.theme,
    )
}
