//! The files every page links: one stylesheet (the theme's tokens, then
//! the layout), one script, the search index. Each is named by its
//! content (`site.3f2a9c1d0b7e4a65.css`) under `_site/`, which is what
//! lets the server keep them for a year
//! (`architect::host::cache_policy`).

use std::fmt::Write as _;

use architect_theme::builtin::fallback;
use architect_theme::{Registry, Scheme, resolve};

/// The layout's stylesheet. It reads only the theme's tokens
/// (`var(--background)`), so it follows any theme.
pub const SITE_CSS: &str = include_str!("../assets/site.css");

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
#[must_use]
pub fn site_js() -> &'static str {
    static JS: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    JS.get_or_init(|| {
        format!(
            "{}\n{}\n{}",
            include_str!("../assets/site.js"),
            classic_rules(),
            include_str!("../assets/player.js")
        )
    })
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
    fn fingerprints_differ_by_content() {
        assert_ne!(fingerprint(b"a"), fingerprint(b"b"));
        assert_eq!(fingerprint(b"a").len(), 16);
    }
}
