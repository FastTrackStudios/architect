//! Packaged core extensions as the standard build ships them: a folder per
//! extension in `core/<id>/` beside the binary, checked against its lock
//! before anything in it is used.
//!
//! `cargo xtask core fetch` puts each folder there from the publisher's
//! release, after checking the archive's sha256 (the list's pin) and the
//! publisher's signature over the release's sums. At run time the folder is
//! checked again, against its own `architect.lock.json` (every file the
//! page loads, with its `sha256-…` digest, base64, as `build-web` writes
//! it), so a folder edited after the fetch is refused whole:
//! - the folder, the lock or a file the lock names is not a regular file
//!   (or, for the folder, a directory) of its own: links, folders and
//!   special files are refused, never followed;
//! - the lock is unreadable, names nothing, names a path that is not a plain
//!   file name in the folder, or does not name the stylesheet it declares;
//! - a file it names is missing, or its bytes are not its digest.
//!
//! Only the files the lock names (and the lock) are served. The check is
//! `architect_sites::package::files`, the one a node's web packages
//! (`ARCHITECT_WEB_PACKAGES`) are held to.
//!
//! Today the one use is a share-page island ([`Island`],
//! `Distribution::island`). Bundled `.archext` packages (present on first
//! run, updated through the registry) are #631's next step.

use std::path::Path;

use architect_sites::SiteIsland;
/// The folder check, the lock's name and why a folder is not used: the
/// one a node's web packages are held to (`architect_sites::package`).
pub use architect_sites::package::{LOCK, PackageError as PackagedError, files};

/// A packaged core extension this build uses, as the list names it
/// (generated into `list.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Packaged {
    /// Its id: its folder is `core/<id>/`.
    pub id: &'static str,
    /// The release it was fetched from.
    pub version: &'static str,
    /// It is a share-page island.
    pub island: Option<Island>,
}

/// A share-page island's views and stylesheet (`SiteIsland`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Island {
    pub progress: Option<&'static str>,
    pub transport: Option<&'static str>,
    pub lyrics: Option<&'static str>,
    pub stylesheet: Option<&'static str>,
}

/// `packaged`'s island from `core/<id>/`, if it is one, is there and
/// matches its lock. A folder that does not match is refused with one
/// warning, and the node serves no island for it (a share link's
/// now-playing view keeps Architect's own bar and controls).
#[must_use]
pub fn island(core: &Path, packaged: &Packaged) -> Option<SiteIsland> {
    let views = packaged.island?;
    let dir = core.join(packaged.id);
    match files(&dir, views.stylesheet) {
        Ok(files) => Some(with_views(SiteIsland::new(packaged.id, files), views)),
        Err(PackagedError::Absent(_)) => None,
        Err(error) => {
            tracing::warn!(
                core.id = packaged.id,
                core.version = packaged.version,
                core.outcome = "refused",
                core.reason = %error,
                "a packaged core extension does not match its lock; not serving it"
            );
            None
        }
    }
}

fn with_views(island: SiteIsland, views: Island) -> SiteIsland {
    let island = match views.progress {
        Some(v) => island.progress(v),
        None => island,
    };
    let island = match views.transport {
        Some(v) => island.transport(v),
        None => island,
    };
    let island = match views.lyrics {
        Some(v) => island.lyrics(v),
        None => island,
    };
    match views.stylesheet {
        Some(s) => island.stylesheet(s),
        None => island,
    }
}
