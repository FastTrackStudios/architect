//! The files every page links: one stylesheet (the theme's tokens, then
//! the layout), one script, the search index. Each is named by its
//! content (`site.3f2a9c1d0b7e4a65.css`) under `_site/`, which is what
//! lets the server keep them for a year
//! (`architect::host::cache_policy`).

use std::borrow::Cow;
use std::fmt::Write as _;

use architect_theme::builtin::fallback;
use architect_theme::{Registry, Scheme, resolve};

/// The layout's stylesheet. It reads only the theme's tokens
/// (`var(--background)`), so it follows any theme.
pub const SITE_CSS: &str = include_str!("../assets/site.css");

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
    dev_asset("site.css").map_or(Cow::Borrowed(SITE_CSS), Cow::Owned)
}

/// What a reader does on the page: search, the theme toggle, the phone
/// menu, the contents' current heading; and the player (#511): media,
/// a share link's queue, and the playhead blocks follow
/// (`assets/player.js`, inert on a page with neither).
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
        return Cow::Owned(format!(
            "{}\n{}\n{}",
            site.as_deref().unwrap_or(include_str!("../assets/site.js")),
            classic_rules(),
            player
                .as_deref()
                .unwrap_or(include_str!("../assets/player.js"))
        ));
    }
    Cow::Borrowed(JS.get_or_init(|| {
        format!(
            "{}\n{}\n{}",
            include_str!("../assets/site.js"),
            classic_rules(),
            include_str!("../assets/player.js")
        )
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
