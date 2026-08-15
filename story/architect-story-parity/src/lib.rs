//! Cross-renderer visual parity diff.
//!
//! Renders the same `Story` through two backends and reports a DSSIM
//! perceptual diff so we can track how closely the **wry desktop**
//! renderer matches the **Blitz native** renderer (or vice versa). The
//! goal isn't pixel-identity — different rasterizers will never match
//! at that bar — but to bound the drift and surface visible regressions.
//!
//! ## How it works
//!
//! - The **Blitz** capture reuses
//!   [`architect_story_snapshots::render_story`] (CPU rasteriser, no display
//!   needed).
//! - The **wry** capture spawns the consuming app's binary in
//!   `--snapshot=<story>` mode against an internally-managed `Xvfb`
//!   display, polls `xdotool` until the window is mapped, screenshots
//!   it via ImageMagick `import`, then kills both processes.
//!
//! Required system tools (Linux only for now): `Xvfb`, `xdotool`,
//! `import` (from imagemagick). Document via the `wry_capture` errors.
//!
//! ## Usage
//!
//! ```ignore
//! use architect_story_parity::{ParityConfig, ParityRunner};
//!
//! #[test]
//! fn button_primary_parity() {
//!     let cfg = ParityConfig {
//!         binary: "target/release/architect-ui-showcase-desktop".into(),
//!         output_dir: "parity_output".into(),
//!         threshold: 0.05,
//!         ..ParityConfig::default()
//!     };
//!     let runner = ParityRunner::new(cfg);
//!     let report = runner
//!         .compare(&architect_ui::stories::BUTTON_PRIMARY_STORY)
//!         .unwrap();
//!     assert!(report.passed, "{report:?}");
//! }
//! ```

pub mod capture;
pub mod diff;
pub mod runner;

pub use capture::{capture_wry_via_xvfb, WryCaptureConfig, WryCaptureError};
pub use diff::{diff_renderers, ParityReport};
pub use runner::{ParityConfig, ParityError, ParityRunner};
