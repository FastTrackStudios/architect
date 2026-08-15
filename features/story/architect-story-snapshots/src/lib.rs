//! Headless visual regression runner backed by Blitz.
//!
//! Public surface:
//!
//! - [`render::render_story`] — given a `&'static Story` and a knob
//!   snapshot, produce a PNG-encoded screenshot via Blitz's CPU
//!   rasteriser. Builds a `DioxusDocument`, runs a settling loop, paints
//!   into an `anyrender_vello_cpu` buffer, encodes via the `png` crate.
//! - [`diff::compare`] — DSSIM perceptual diff of two PNGs against a
//!   threshold.
//! - [`harness::assert_snapshot`] — convenience used inside `#[test]`
//!   functions: render the story with its default knobs, compare against
//!   `snapshots/<category>__<name>.png`, write a candidate PNG to
//!   `diff_output/` on mismatch, panic with a useful message.
//!
//! ⚠ Stylo/Parley produce *incorrect* renders under `debug_assertions`.
//! [`render::render_story`] panics if invoked in a debug build so the
//! failure is loud — run snapshot tests with `cargo test --release` (or
//! the supplied Docker harness in `docker/`).

pub mod diff;
pub mod harness;
pub mod render;

pub use diff::{compare, DiffError};
pub use harness::{assert_snapshot, SnapshotConfig};
pub use render::{clear_wrapper, install_wrapper, render_story, RenderConfig};

/// Re-exported so consumers can build a knob snapshot without depending
/// on `architect-story-runtime` directly.
pub use architect_story_runtime::{KnobSource, KnobValue, NoKnobs, Story, STORIES};
