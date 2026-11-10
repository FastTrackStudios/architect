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
pub const SITE_JS: &str = concat!(
    include_str!("../assets/site.js"),
    "\n",
    include_str!("../assets/player.js")
);

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
    fn fingerprints_differ_by_content() {
        assert_ne!(fingerprint(b"a"), fingerprint(b"b"));
        assert_eq!(fingerprint(b"a").len(), 16);
    }
}
