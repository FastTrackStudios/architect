//! The playhead as a process: the same `Playhead` an app links in, served
//! over vox to the host that started it (`extension.toml`'s `[entry]`),
//! as a development copy does (`EXT_CLONE {"id":"playhead"}`). Its deck is
//! its own process's.

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_playhead::Playhead).serve()
}
