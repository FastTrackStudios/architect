//! The files every page links: one stylesheet (the theme's tokens, then
//! the layout), one script, the search index. Each is named by its
//! content (`site.3f2a9c1d0b7e4a65.css`) under `_site/`, which is what
//! lets the server keep them for a year
//! (`architect::host::cache_policy`).

use std::borrow::Cow;
use std::fmt::Write as _;

use architect_theme::builtin::fallback;
use architect_theme::{Registry, Scheme, resolve};

/// The layout's stylesheet as written: the phone's layout under
/// `@media (--site-phone)` (a phone on its side `(--site-phone-flat)`), the
/// desk's under `(--site-not-phone)` and `(--site-desk)`, which
/// [`site_css`] fills in. It reads only the theme's tokens
/// (`var(--background)`), so it follows any theme.
pub const SITE_CSS: &str = include_str!("../assets/site.css");

/// When a page takes the phone's layout (and a share link its phone shell).
///
/// Narrow (every phone upright, a small tablet upright), or a touch screen
/// too short to be anything but a phone on its side. The one place it is
/// said: [`site_css`] and [`site_js`] put it where the sheet and the
/// scripts write `(--site-phone)`, and the scripts' test harness reads it
/// from here (`features/sites/test/player-playlist.test.mjs`).
///
/// - `(max-width: 820px)`: the width the layout always switched at (every
///   phone upright; an iPad Air upright is 820).
/// - `(pointer: coarse) and (max-height: 500px)`: a phone turned (its
///   height 320 to 440 CSS px). A tablet turned is 744 or more, and a
///   desk's short window has a fine pointer, so neither is one.
pub const PHONE: &str = "(max-width: 820px), (pointer: coarse) and (max-height: 500px)";

/// A phone on its side ([`PHONE`]'s second clause): a chart read on a
/// stand, so the bars give it the height.
pub const PHONE_FLAT: &str = "(pointer: coarse) and (max-height: 500px)";

/// Not a phone ([`PHONE`] negated): wider than 820, and not a touch screen
/// 500 high or less.
pub const NOT_PHONE: &str = "(min-width: 821px) and (pointer: fine), (min-width: 821px) and (pointer: none), (min-width: 821px) and (min-height: 501px)";

/// A desk (the player's side panel, a row menu beside its button): 900
/// wide or more, and not a phone on its side.
pub const DESK: &str = "(min-width: 900px) and (pointer: fine), (min-width: 900px) and (pointer: none), (min-width: 900px) and (min-height: 501px)";

/// `text` with the layout's media queries filled in.
fn layout_queries(text: &str) -> String {
    text.replace("(--site-phone-flat)", PHONE_FLAT)
        .replace("(--site-not-phone)", NOT_PHONE)
        .replace("(--site-desk)", DESK)
        .replace("(--site-phone)", PHONE)
}

/// Names a folder a debug build reads `site.css`, `site.js` and
/// `player.js` from, in place of the copies compiled in: each build of a
/// site reads them again, so an edit to `features/sites/assets` shows on
/// a node's next build of its pages without relinking the node (Studio's
/// local stack sets it to a local Architect checkout's assets). A release
/// build ignores it.
pub const DEV_ASSETS_VAR: &str = "ARCHITECT_SITES_ASSETS_DIR";

/// `name` from [`DEV_ASSETS_VAR`]'s folder, in a debug build that names
/// one; `None` otherwise (and for a file that is not there).
fn dev_asset(name: &str) -> Option<String> {
    dev_asset_in(std::env::var_os(DEV_ASSETS_VAR).as_deref(), name)
}

fn dev_asset_in(dir: Option<&std::ffi::OsStr>, name: &str) -> Option<String> {
    if !cfg!(debug_assertions) {
        return None;
    }
    std::fs::read_to_string(std::path::Path::new(dir?).join(name)).ok()
}

/// The layout's stylesheet: [`SITE_CSS`], or a debug build's
/// [`DEV_ASSETS_VAR`] copy.
#[must_use]
pub fn site_css() -> Cow<'static, str> {
    static CSS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    dev_asset("site.css").map_or_else(
        || Cow::Borrowed(CSS.get_or_init(|| layout_queries(SITE_CSS)).as_str()),
        |css| Cow::Owned(layout_queries(&css)),
    )
}

/// What a reader does on the page: search, the theme toggle, the phone
/// menu, the contents' current heading; and the player (#511): media,
/// a share link's queue, and the playhead blocks follow
/// (`assets/player.js`, inert on a page with neither); a share link's
/// phone shell (`assets/shell.js`, inert without the node's `site-shell`).
///
/// Between them, the share pages' allow-lists as the player and every
/// embedder of the web runtime hold to them (`features/extensions/web/
/// sanitize.js`, one source): what the now-playing view brings in from
/// another page is cleaned with them on every Site and share link, with
/// or without the node's runtime (studio#445).
///
/// A debug build reads `site.js` and `player.js` from [`DEV_ASSETS_VAR`]'s
/// folder when it names one, at every call.
#[must_use]
pub fn site_js() -> Cow<'static, str> {
    static JS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let (site, player) = (dev_asset("site.js"), dev_asset("player.js"));
    if site.is_some() || player.is_some() {
        return Cow::Owned(layout_queries(&format!(
            "{}\n{}\n{}",
            site.as_deref().unwrap_or(include_str!("../assets/site.js")),
            classic_rules(),
            player
                .as_deref()
                .unwrap_or(include_str!("../assets/player.js"))
        )));
    }
    Cow::Borrowed(JS.get_or_init(|| {
        layout_queries(&format!(
            "{}\n{}\n{}\n{}",
            include_str!("../assets/site.js"),
            classic_rules(),
            include_str!("../assets/player.js"),
            include_str!("../assets/shell.js")
        ))
    }))
}

/// `sanitize.js` (an ES module, no imports) as a classic script: its
/// `export`s taken off, its body in a function of its own, what the
/// player uses put on `architect.rules`.
fn classic_rules() -> String {
    let module = include_str!("../../extensions/web/sanitize.js");
    let body: String = module
        .lines()
        .map(|line| line.strip_prefix("export ").unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "(function () {{\n\"use strict\";\n{body}\nvar a = (window.architect = window.architect || {{}});\nif (!a.rules) a.rules = Object.freeze({{ elementRule: elementRule, attrRule: attrRule, cssInert: cssInert, cleanSheet: cleanSheet }});\n}})();"
    )
}

/// The theme family `theme`'s light and dark tokens as CSS variables: the
/// system picks one, and a reader's choice (`data-theme` on `<html>`, set
/// by the toggle) overrides it. An unknown family is architect's default.
#[must_use]
pub fn theme_css(theme: &str) -> String {
    let registry = Registry::builtin();
    let tokens = |scheme: Scheme, suffix: &str| {
        let chosen = registry
            .get(&format!("{theme}-{suffix}"))
            .cloned()
            .unwrap_or_else(|| fallback(scheme));
        let resolved = resolve(&chosen, &registry);
        let mut out = format!("  color-scheme: {suffix};\n");
        for (name, value) in resolved.css_tokens() {
            let _ = writeln!(out, "  --{name}: {value};");
        }
        out
    };
    let light = tokens(Scheme::Light, "light");
    let dark = tokens(Scheme::Dark, "dark");
    format!(
        ":root, :root[data-theme=\"light\"] {{\n{light}}}\n\
         :root[data-theme=\"dark\"] {{\n{dark}}}\n\
         @media (prefers-color-scheme: dark) {{\n:root:not([data-theme=\"light\"]) {{\n{dark}}}\n}}\n"
    )
}

/// A content hash for a file name: FNV-1a, 64 bits, as hex. Not for
/// security, only so a changed file gets a new name.
#[must_use]
pub fn fingerprint(bytes: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One definition of a phone: the sheet and the scripts as served carry
    /// [`PHONE`] (and the desk's queries) where they write `(--site-…)`,
    /// and nothing of the placeholder is left.
    #[test]
    fn the_sheet_and_the_scripts_share_one_phone() {
        for served in [site_css(), site_js()] {
            for token in [
                "(--site-phone)",
                "(--site-phone-flat)",
                "(--site-not-phone)",
                "(--site-desk)",
            ] {
                assert!(!served.contains(token), "{token} left");
            }
        }
        assert!(site_css().contains(&format!("@media {PHONE} {{")));
        assert!(site_css().contains(&format!("@media {DESK} {{")));
        assert!(site_css().contains(&format!("@media {NOT_PHONE} {{")));
        assert!(site_js().contains(&format!("matchMedia(\"{PHONE}\")")));
        assert!(site_js().contains(&format!("matchMedia(\"{DESK}\")")));
        assert!(
            !SITE_CSS.contains("max-width: 820px)"),
            "the width is said once, in PHONE"
        );
    }

    #[test]
    fn both_schemes_carry_the_tokens() {
        let css = theme_css("fasttrackstudio");
        assert!(css.contains("--background:"), "{css}");
        assert_eq!(css.matches("  color-scheme: dark;").count(), 2);
        // An unknown family falls back rather than writing nothing.
        assert!(theme_css("nope").contains("--foreground:"));
    }

    #[test]
    fn the_site_script_carries_the_share_pages_rules_for_its_player() {
        let js = site_js();
        let rules = js.find("a.rules = Object.freeze").unwrap();
        let player = js.find("architect-sites: the player").unwrap();
        assert!(rules < player, "the rules before the player");
        assert!(
            !js.lines().any(|l| l.starts_with("export ")),
            "a classic script"
        );
        assert!(js.contains("function cleanSheet(css"));
    }

    #[test]
    fn a_debug_build_reads_the_dev_assets_folder() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("player.js"), "// edited").unwrap();
        let dir = Some(tmp.path().as_os_str());
        assert_eq!(
            dev_asset_in(dir, "player.js").as_deref(),
            cfg!(debug_assertions).then_some("// edited")
        );
        // Not there, or no folder named: the compiled-in copy.
        assert_eq!(dev_asset_in(dir, "site.css"), None);
        assert_eq!(dev_asset_in(None, "player.js"), None);
    }

    #[test]
    fn fingerprints_differ_by_content() {
        assert_ne!(fingerprint(b"a"), fingerprint(b"b"));
        assert_eq!(fingerprint(b"a").len(), 16);
    }
}
