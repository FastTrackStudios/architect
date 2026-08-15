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
    pub fn new(cfg: ParityConfig) -> Self {
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
        blitz_render.width = (self.cfg.wry_capture.screen_w as f32 / blitz_render.scale) as u32;
        blitz_render.height = (self.cfg.wry_capture.screen_h as f32 / blitz_render.scale) as u32;

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
    match story.category {
        Some(c) => format!("{}__{}", sanitise(c), sanitise(story.name)),
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
