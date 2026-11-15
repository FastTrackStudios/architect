//! Review as a process: the same `Review` an app links in, served over
//! vox to the host that started it, as a development copy does
//! (`EXT_CLONE {"id":"review"}`).

use architect_extensions::guest::{Guest, GuestError};

fn main() -> Result<(), GuestError> {
    Guest::extension(&architect_review::Review).serve()
}
