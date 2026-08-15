//! Cross-renderer parity tests: Blitz native ⇄ wry desktop.
//!
//! Requires the **release** desktop binary to exist before running:
//!
//! ```sh
//! cargo build --release -p architect-ui-showcase-desktop
//! cargo test  --release -p architect-ui --test parity
//! ```
//!
//! Outputs land in `crates/architect-ui/parity_output/` — one PNG per
//! renderer plus a side-by-side composite for human review.
//!
//! Headless: spawns its own `Xvfb` instance (display `:99` by
//! default). Required system tools: `Xvfb`, `xdotool`, ImageMagick
//! (`import`). The test will fail-fast with a useful error message if
//! any are missing.

use architect_story_parity::{ParityConfig, ParityRunner, WryCaptureConfig};
use architect_ui::prelude::*;
use architect_ui::stories;
use dioxus::prelude::*;
use std::path::PathBuf;

/// Wrap every Blitz-rendered story in a `ThemeProvider` AND inject the
/// pre-compiled Tailwind stylesheet inline. Without the stylesheet
/// Blitz has no rules for `bg-background` / `p-6` / `rounded-md` /
/// etc., so the rendered Element collapses to default browser styling
/// even with the right CSS variables defined.
///
/// The wry side gets its stylesheet through
/// `document::Stylesheet { href: TAILWIND_CSS }` in
/// `apps/desktop/src/main.rs`; the Blitz snapshot path doesn't go
/// through dx so we load the same compiled file here.
///
/// Read at RUNTIME, not `include_str!`: the sheet is generated,
/// gitignored build output (`just css` in apps/task/), and a
/// compile-time include made this whole test binary — and with it
/// `cargo test -p architect-ui` on a fresh checkout — fail to build until an
/// unrelated app had been compiled once.
fn tailwind_css() -> &'static str {
    static SHEET: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    SHEET.get_or_init(|| {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../apps/task/desktop/assets/tailwind.css"
        );
        std::fs::read_to_string(path).unwrap_or_else(|e| {
            panic!(
                "{path}: {e}\n\
                 The sheet is generated build output. Produce it with:\n  \
                 cd apps/task && just css"
            )
        })
    })
}

fn theme_wrap(child: Element) -> Element {
    let state = use_signal(|| ThemeState::new(default_theme_preset(), ThemeMode::Dark));
    let sheet = tailwind_css();
    rsx! {
        style { {sheet} }
        ThemeProvider { state, {child} }
    }
}

fn binary_path() -> PathBuf {
    // CARGO_MANIFEST_DIR points at crates/architect-ui; walk up to the
    // workspace root, then into target/release.
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates
    p.pop(); // workspace root
    p.join("target/release/architect-ui-showcase-desktop")
}

fn cfg() -> ParityConfig {
    let mut cfg = ParityConfig::for_crate(env!("CARGO_MANIFEST_DIR"));
    cfg.wry_capture = WryCaptureConfig {
        binary: binary_path(),
        ..WryCaptureConfig::default()
    };
    cfg
}

fn run(story: &'static architect_story_runtime::Story) {
    stories::force_link();
    architect_story_snapshots::install_wrapper(theme_wrap);
    let runner = ParityRunner::new(cfg());
    let report = runner.compare(story).expect("parity run");
    architect_story_snapshots::clear_wrapper();
    println!(
        "parity score for {} = {:.6} (threshold {:.6})",
        report.story, report.dssim_score, report.threshold
    );
    assert!(
        report.passed,
        "wry vs blitz dssim {:.6} > threshold {:.6} for {}",
        report.dssim_score, report.threshold, report.story
    );
}

#[test]
#[ignore = "requires Xvfb/xdotool/imagemagick + a built target/release/architect-ui-showcase-desktop"]
fn button_primary_parity() {
    run(&stories::BUTTON_PRIMARY_STORY);
}

#[test]
#[ignore = "requires Xvfb/xdotool/imagemagick + a built target/release/architect-ui-showcase-desktop"]
fn button_matrix_parity() {
    run(&stories::BUTTON_MATRIX_STORY);
}

#[test]
#[ignore = "requires Xvfb/xdotool/imagemagick + a built target/release/architect-ui-showcase-desktop"]
fn button_sizes_parity() {
    run(&stories::BUTTON_SIZES_STORY);
}
