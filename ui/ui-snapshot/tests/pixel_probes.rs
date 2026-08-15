//! Pixel-sample regression gates for the OKLCH → paint/SVG pipeline.
//!
//! Renders the `theme-probes` scene (see `src/scenes.rs`) and asserts that
//! fixed-coordinate samples fall inside the expected RGB range for each
//! theme token. Complements the golden-PNG diff in `src/main.rs::check` by
//! failing with a precise, color-named error instead of "N pixels differ".
//!
//! Coordinates — the scene is a 400×160 grid of 80×80 blocks. Column
//! centers are x = 40, 120, 200, 280, 360. Row 0 (y=40) exercises
//! `background-color: oklch(...)` inline; row 1 (y=120) exercises
//! `color: oklch(...)` + inline SVG `fill="currentColor"` (the
//! `color_to_svg_compatible` substitution path in blitz-dom).

use ui_snapshot::{SCENES, render_scene, sample_pixel};

/// Theme token → expected sRGB ballpark. Values derived from the
/// `fts-theme.css` `:root` OKLCH definitions, converted through stylo's
/// sRGB gamut mapping. Tolerance is ±6 per channel — tight enough to
/// detect "color silently became white/black", loose enough to absorb
/// minor changes in OKLCH→sRGB conversion across stylo versions.
struct Expect {
    label: &'static str,
    r: (u8, u8),
    g: (u8, u8),
    b: (u8, u8),
}

// Ranges established empirically from the first successful render. stylo
// gamut-clips out-of-sRGB OKLCH channels hard to [0, 255], so high-chroma
// tokens like `destructive` land with g/b near 0. If stylo tweaks OKLCH
// conversion in a future bump, loosen the bounds — but if they shift
// dramatically (e.g. red → gray), that's a regression.
const DESTRUCTIVE: Expect = Expect {
    label: "destructive (oklch 0.5757 0.2352 27.92)",
    r: (200, 240),
    g: (0, 60),
    b: (0, 50),
};
const PRIMARY: Expect = Expect {
    label: "primary (oklch 0.205 0 0)",
    r: (10, 80),
    g: (10, 80),
    b: (10, 80),
};
const CHART_2: Expect = Expect {
    label: "chart-2 (oklch 0.6 0.118 184.704)",
    // teal-ish — low red, high green, high blue (cyan leaning).
    r: (0, 80),
    g: (120, 200),
    b: (130, 210),
};
const FOREGROUND: Expect = Expect {
    label: "foreground (oklch 0.145 0 0)",
    r: (0, 60),
    g: (0, 60),
    b: (0, 60),
};
const BACKGROUND: Expect = Expect {
    label: "background (oklch 1 0 0)",
    r: (240, 255),
    g: (240, 255),
    b: (240, 255),
};

fn assert_in(px: [u8; 4], expect: &Expect, where_: &str) {
    let [r, g, b, _] = px;
    let ok = (expect.r.0..=expect.r.1).contains(&r)
        && (expect.g.0..=expect.g.1).contains(&g)
        && (expect.b.0..=expect.b.1).contains(&b);
    assert!(
        ok,
        "{where_}: expected {} (r={}..={}, g={}..={}, b={}..={}), got rgb({r}, {g}, {b})",
        expect.label, expect.r.0, expect.r.1, expect.g.0, expect.g.1, expect.b.0, expect.b.1,
    );
}

fn probe_scene() -> &'static ui_snapshot::Scene {
    SCENES
        .iter()
        .find(|s| s.name == "theme-probes")
        .expect("theme-probes scene registered")
}

#[test]
fn oklch_background_paints_expected_srgb() {
    // Exercises the stylo → blitz-paint `background-color: oklch(...)`
    // path. Independent of the node.rs fix — guards against regressions in
    // the OKLCH parse/convert/paint pipeline itself.
    let scene = probe_scene();
    let buf = render_scene(scene);
    let w = scene.width;

    // Row 0, y = 40; column centers x = 40, 120, 200, 280, 360.
    assert_in(
        sample_pixel(&buf, w, 40, 40),
        &DESTRUCTIVE,
        "bg destructive",
    );
    assert_in(sample_pixel(&buf, w, 120, 40), &PRIMARY, "bg primary");
    assert_in(sample_pixel(&buf, w, 200, 40), &CHART_2, "bg chart-2");
    assert_in(sample_pixel(&buf, w, 280, 40), &FOREGROUND, "bg foreground");
    assert_in(sample_pixel(&buf, w, 360, 40), &BACKGROUND, "bg background");
}

#[test]
#[ignore = "known upstream regression: at blitz 727dab01 (the rev daw main \
builds against) with stylo 0.18, write_outer_html's currentColor substitution \
serializes the cascaded color back as oklch(...); usvg 0.45 drops the \
unparseable fill and the SVG paints black. The older 9ebd23a5 graph this gate \
passed with carried the color_to_svg_compatible behavior. Re-enable when the \
blitz pin advances to a rev that serializes currentColor svg-compatibly."]
fn svg_currentcolor_under_oklch_cascade() {
    // Regression gate for the blitz-dom `color_to_svg_compatible` fix.
    // Before the fix, a parent `color: oklch(...)` would serialize back to
    // `oklch(...)` via stylo's default to_css_string; usvg 0.45 can't
    // parse that and drops the `fill="currentColor"` attribute, leaving a
    // transparent rect (which paints as the background color — white).
    // Row 1 samples must match row 0 one-to-one because they use the same
    // OKLCH values — row 0 via `background-color`, row 1 via
    // `color` + `fill="currentColor"`.
    let scene = probe_scene();
    let buf = render_scene(scene);
    let w = scene.width;

    // Row 1, y = 120; same column centers as row 0.
    assert_in(
        sample_pixel(&buf, w, 40, 120),
        &DESTRUCTIVE,
        "svg fill=currentColor under color=oklch(destructive)",
    );
    assert_in(
        sample_pixel(&buf, w, 120, 120),
        &PRIMARY,
        "svg fill=currentColor under color=oklch(primary)",
    );
    assert_in(
        sample_pixel(&buf, w, 200, 120),
        &CHART_2,
        "svg fill=currentColor under color=oklch(chart-2)",
    );
    assert_in(
        sample_pixel(&buf, w, 280, 120),
        &FOREGROUND,
        "svg fill=currentColor under color=oklch(foreground)",
    );
    assert_in(
        sample_pixel(&buf, w, 360, 120),
        &BACKGROUND,
        "svg fill=currentColor under color=oklch(background)",
    );
}
