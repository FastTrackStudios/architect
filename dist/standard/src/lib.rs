//! The standard distribution of Architect (#631,
//! `docs/content/guide/core-extensions.md`).
//!
//! The Architect app (`architect_desktop::distribution::architect()`: its
//! name, bundle id, folder and first-party extensions) with the **core
//! extensions** that `extensions/core-extensions.toml` lists:
//! - each `linked` one compiled in from its own repo at the tag the list
//!   pins (`list::linked`, generated from the list by
//!   `cargo xtask core sync`);
//! - each `packaged` one read from `core/<id>/` beside the binary, where
//!   `cargo xtask core fetch` puts what it fetched and checked from the
//!   publisher's release ([`packaged`]).
//!
//! This crate is outside architect's workspace: its own `Cargo.toml`
//! (generated) and lock, with every architect crate in its graph patched
//! back to this checkout, so the build holds one Architect. Nothing in the
//! workspace depends on it.

mod list;
pub mod packaged;

use std::path::Path;

use architect_desktop::Distribution;

pub use crate::list::PACKAGED;

/// Where packaged core extensions ship: `core/<id>/` beside the binary.
pub const CORE_FOLDER: &str = "core";

/// The standard Architect: the Architect app with every core extension,
/// its packaged ones read from [`CORE_FOLDER`] beside the running binary.
#[must_use]
pub fn distribution() -> Distribution {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join(CORE_FOLDER)));
    distribution_in(dir.as_deref())
}

/// [`distribution`] with its packaged core extensions read from `core`
/// (none: only the linked ones).
#[must_use]
pub fn distribution_in(core: Option<&Path>) -> Distribution {
    let dist = crate::list::linked(architect_desktop::distribution::architect());
    let Some(core) = core else {
        return dist;
    };
    PACKAGED
        .iter()
        .filter_map(|p| packaged::island(core, p))
        .fold(dist, Distribution::island)
}
