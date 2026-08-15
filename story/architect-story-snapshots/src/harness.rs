//! Test harness — call from inside `#[test]` fns to assert a story
//! against its committed baseline.
//!
//! ```ignore
//! use architect_story_snapshots::{assert_snapshot, SnapshotConfig};
//!
//! #[test]
//! fn button_primary_matches_baseline() {
//!     let cfg = SnapshotConfig::for_crate(env!("CARGO_MANIFEST_DIR"));
//!     assert_snapshot(&architect_ui::stories::BUTTON_PRIMARY_STORY, &cfg);
//! }
//! ```
//!
//! On first run a baseline doesn't exist; the harness writes the
//! current render as the new baseline and the test passes. To force a
//! refresh, set `FTS_STORY_UPDATE_SNAPSHOTS=1`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use architect_story_runtime::{KnobValue, Story};

use crate::{compare, render_story, RenderConfig};

/// Where to find baselines + where to write candidates on failure.
#[derive(Clone, Debug)]
pub struct SnapshotConfig {
    pub baseline_dir: PathBuf,
    pub diff_dir: PathBuf,
    pub render: RenderConfig,
    /// DSSIM threshold (0.0 = pixel-perfect, 0.001 = perceptual).
    pub threshold: f64,
}

impl SnapshotConfig {
    /// Standard layout: `<crate>/snapshots/` for baselines,
    /// `<crate>/diff_output/` for candidates on failure.
    pub fn for_crate(manifest_dir: impl AsRef<Path>) -> Self {
        let root = manifest_dir.as_ref();
        Self {
            baseline_dir: root.join("snapshots"),
            diff_dir: root.join("diff_output"),
            render: RenderConfig::default(),
            threshold: 0.001,
        }
    }

    pub fn with_render(mut self, render: RenderConfig) -> Self {
        self.render = render;
        self
    }

    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = threshold;
        self
    }
}

/// Assert that `story`, rendered with its declared default knob
/// values, matches the committed baseline.
///
/// Writes the candidate PNG to `cfg.diff_dir` on mismatch and panics
/// with a useful message. On first run (no baseline file) writes the
/// current render as the baseline and returns successfully.
pub fn assert_snapshot(story: &'static Story, cfg: &SnapshotConfig) {
    let knobs = default_knob_map(story);
    let png = render_story(story, knobs, &cfg.render);
    let label = snapshot_label(story);
    let baseline_path = cfg.baseline_dir.join(format!("{label}.png"));

    let force_update = std::env::var("FTS_STORY_UPDATE_SNAPSHOTS")
        .map(|v| v == "1")
        .unwrap_or(false);

    if force_update || !baseline_path.exists() {
        if let Some(parent) = baseline_path.parent() {
            std::fs::create_dir_all(parent).expect("baseline dir");
        }
        std::fs::write(&baseline_path, &png).expect("write baseline");
        if force_update {
            eprintln!("architect-story-snapshots: refreshed {}", baseline_path.display());
        } else {
            eprintln!("architect-story-snapshots: created {}", baseline_path.display());
        }
        return;
    }

    let baseline = std::fs::read(&baseline_path).expect("read baseline");
    match compare(&baseline, &png, cfg.threshold) {
        Ok(score) => {
            // Helpful breadcrumb for near-threshold runs.
            if score > cfg.threshold * 0.5 {
                eprintln!(
                    "architect-story-snapshots: {label} matched (dssim={score:.6}, threshold={:.6})",
                    cfg.threshold
                );
            }
        }
        Err(err) => {
            std::fs::create_dir_all(&cfg.diff_dir).expect("diff dir");
            let candidate_path = cfg.diff_dir.join(format!("{label}.candidate.png"));
            std::fs::write(&candidate_path, &png).expect("write candidate");
            panic!(
                "snapshot mismatch for `{label}`: {err}\n\
                 baseline:  {}\n\
                 candidate: {}\n\
                 Re-run with `FTS_STORY_UPDATE_SNAPSHOTS=1 cargo test --release` \
                 to regenerate the baseline if this change is intentional.",
                baseline_path.display(),
                candidate_path.display(),
            );
        }
    }
}

fn snapshot_label(story: &Story) -> String {
    match story.category {
        Some(category) => format!("{}__{}", sanitise(category), sanitise(story.name)),
        None => sanitise(story.name).into_owned(),
    }
}

fn sanitise(s: &str) -> std::borrow::Cow<'_, str> {
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        std::borrow::Cow::Borrowed(s)
    } else {
        std::borrow::Cow::Owned(
            s.chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                        c
                    } else {
                        '_'
                    }
                })
                .collect(),
        )
    }
}

fn default_knob_map(story: &Story) -> HashMap<&'static str, KnobValue> {
    story
        .knobs
        .iter()
        .filter_map(|spec| spec.default.clone().map(|v| (spec.name, v)))
        .collect()
}
