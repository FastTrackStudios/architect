//! End-to-end orchestration: render via Blitz, capture via wry+Xvfb,
//! diff, write outputs.

use std::collections::HashMap;
use std::path::PathBuf;

use architect_story_runtime::{KnobValue, Story};
use architect_story_snapshots::{render_story, RenderConfig};

use crate::capture::{capture_wry_via_xvfb, write_png, WryCaptureConfig, WryCaptureError};
use crate::diff::{diff_renderers, ParityReport};

#[derive(Debug, thiserror::Error)]
pub enum ParityError {
    #[error("missing system dependency: {0}")]
    MissingDeps(String),
    #[error("wry capture failed: {0}")]
    WryCapture(#[from] WryCaptureError),
    #[error("diff failed: {0}")]
    Diff(#[from] architect_story_snapshots::DiffError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct ParityConfig {
    /// Where to drop `<story>.blitz.png`, `<story>.wry.png`,
    /// `<story>.composite.png` after each run.
    pub output_dir: PathBuf,
    /// DSSIM threshold above which `report.passed = false`. Cross-
    /// renderer diffs sit naturally around `0.02`; bump if you have
    /// gradient-heavy stories.
    pub threshold: f64,
    /// Knob overrides applied to the Blitz render. Defaults are used
    /// for any unsupplied knob. The wry side currently always uses
    /// declared defaults — overriding on that side requires an IPC
    /// channel into the desktop binary which isn't built yet.
    pub blitz_knobs: HashMap<&'static str, KnobValue>,
    /// Forwarded to the Blitz render path.
    pub blitz_render: RenderConfig,
    /// Forwarded to the wry capture path.
    pub wry_capture: WryCaptureConfig,
}

impl ParityConfig {
    pub fn for_crate(manifest_dir: impl Into<PathBuf>) -> Self {
        let dir: PathBuf = manifest_dir.into();
        Self {
            output_dir: dir.join("parity_output"),
            threshold: 0.05,
            blitz_knobs: HashMap::new(),
            blitz_render: RenderConfig::default(),
            wry_capture: WryCaptureConfig::default(),
        }
    }
}

pub struct ParityRunner {
    cfg: ParityConfig,
}

impl ParityRunner {
    #[must_use]
    pub const fn new(cfg: ParityConfig) -> Self {
        Self { cfg }
    }

    /// Render `story` via both renderers, diff, write artefacts.
    pub fn compare(&self, story: &'static Story) -> Result<ParityReport, ParityError> {
        crate::capture::check_dependencies().map_err(ParityError::MissingDeps)?;

        // Make sure the Blitz render matches the wry window so the
        // PNGs are dimensionally comparable. The wry window is at
        // device pixels (no HiDPI scaling); Blitz multiplies by
        // `scale`, so back the CSS-pixel size out from the screen
        // dimensions.
        let mut blitz_render = self.cfg.blitz_render.clone();
        blitz_render.width = css_px(self.cfg.wry_capture.screen_w, blitz_render.scale);
        blitz_render.height = css_px(self.cfg.wry_capture.screen_h, blitz_render.scale);

        let blitz_png = render_story(story, self.cfg.blitz_knobs.clone(), &blitz_render);
        let wry_png = capture_wry_via_xvfb(&self.cfg.wry_capture, story.name)?;

        let report = diff_renderers(story.name, blitz_png, wry_png, self.cfg.threshold)?;

        // Persist artefacts.
        let label = label_for(story);
        let dir = &self.cfg.output_dir;
        write_png(&report.blitz_png, dir.join(format!("{label}.blitz.png")))?;
        write_png(&report.wry_png, dir.join(format!("{label}.wry.png")))?;
        write_png(
            &report.composite_png,
            dir.join(format!("{label}.composite.png")),
        )?;
        Ok(report)
    }
}

fn label_for(story: &Story) -> String {
    story.category.map_or_else(
        || sanitise(story.name).into_owned(),
        |c| format!("{}__{}", sanitise(c), sanitise(story.name)),
    )
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

/// Back a CSS-pixel dimension out of a device-pixel one.
///
/// `device / scale`, clamped into `u32`. A zero or non-finite scale (a
/// mis-set config) yields the device size rather than a NaN cast, which
/// `as` would silently turn into `0`.
// f64 -> u32 has no total conversion in std. Everything that could make
// the cast lossy is excluded above: non-finite and negative values return
// early, and `min` caps at `u32::MAX`.
#[allow(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn css_px(device: u32, scale: f32) -> u32 {
    if !scale.is_finite() || scale <= 0.0 {
        return device;
    }
    let px = f64::from(device) / f64::from(scale);
    if px.is_finite() && px >= 0.0 {
        // `px` is a pixel count already clamped to a sane range below.
        px.min(f64::from(u32::MAX)).round() as u32
    } else {
        device
    }
}
