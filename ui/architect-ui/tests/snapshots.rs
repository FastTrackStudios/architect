//! Snapshot regression tests for architect-ui stories.
//!
//! Run with `cargo test --release -p architect-ui`.
//! In debug mode `render_story` panics — Stylo/Parley produce
//! incorrect output under `debug_assertions`.
//!
//! On first run, baselines are written to `crates/architect-ui/snapshots/`.
//! On mismatch, a candidate PNG is written to
//! `crates/architect-ui/diff_output/` and the test panics with both paths.
//! To accept current renders as new baselines:
//! `FTS_STORY_UPDATE_SNAPSHOTS=1 cargo test --release -p architect-ui`.

use architect_story_snapshots::{assert_snapshot, SnapshotConfig};
use architect_ui::stories;

fn cfg() -> SnapshotConfig {
    SnapshotConfig::for_crate(env!("CARGO_MANIFEST_DIR"))
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "VRT snapshot: render is only valid in release; run `cargo test --release -p architect-ui`"
)]
fn button_primary_default() {
    // Touch force_link so other registrations also pull in cleanly when
    // this test file is the only consumer of the stories module.
    stories::force_link();
    assert_snapshot(&stories::BUTTON_PRIMARY_STORY, &cfg());
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "VRT snapshot: render is only valid in release; run `cargo test --release -p architect-ui`"
)]
fn button_variants_default() {
    assert_snapshot(&stories::BUTTON_VARIANTS_STORY, &cfg());
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "VRT snapshot: render is only valid in release; run `cargo test --release -p architect-ui`"
)]
fn badge_variants_default() {
    assert_snapshot(&architect_ui::components::BADGE_VARIANTS_STORY, &cfg());
}

#[test]
#[cfg_attr(
    debug_assertions,
    ignore = "VRT snapshot: render is only valid in release; run `cargo test --release -p architect-ui`"
)]
fn card_basic_default() {
    assert_snapshot(&stories::CARD_BASIC_STORY, &cfg());
}
