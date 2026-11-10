//! Highlight colours as theme tokens (`syntax.keyword`, `syntax.string`,
//! …), so every theme styles every language.
//!
//! An app adds [`TOKENS`] to its `App` layer
//! (`architect_theme::Layer::token`) and lists [`NAMESPACE`] among the
//! namespaces its theme writes as CSS variables
//! (`architect_theme::Resolved::css_namespaced`: `--syntax-keyword`). A
//! theme file, an extension's `[tokens]` or a person overrides any of
//! them. [`css`] is the sheet that points the editor's token classes at
//! those variables; it goes after the editor's own (`ui::EDITOR_CSS`).

/// The tokens' namespace: every name starts `syntax.`.
pub const NAMESPACE: &str = "syntax";

/// One highlight role: its theme token ([`TOKENS`] has its default) and the
/// colour it is drawn in when the app left the tokens out (the editor's
/// own).
struct Role {
    token: &'static str,
    fallback: &'static str,
    /// The scope tags it colours (`editor-syntax`'s `Token::tag`, the
    /// editor's `md-tok-<tag>` classes).
    tags: &'static [&'static str],
}

const ROLES: &[Role] = &[
    Role {
        token: "syntax.keyword",
        fallback: "color-mix(in oklab, #c792ea 65%, currentColor)",
        tags: &[
            "k", "kc", "kd", "ke", "kf", "ki", "km", "ko", "kp", "kr", "kt", "ky",
        ],
    },
    Role {
        token: "syntax.string",
        fallback: "color-mix(in oklab, #c3e88d 55%, currentColor)",
        tags: &["s", "sc", "se", "sp", "ss", "st", "rx"],
    },
    Role {
        token: "syntax.comment",
        fallback: "color-mix(in oklab, currentColor 55%, transparent)",
        tags: &["c", "cb"],
    },
    Role {
        token: "syntax.number",
        fallback: "color-mix(in oklab, #f78c6c 65%, currentColor)",
        tags: &["n"],
    },
    Role {
        token: "syntax.constant",
        fallback: "color-mix(in oklab, #ff5370 65%, currentColor)",
        tags: &["co", "b"],
    },
    Role {
        token: "syntax.function",
        fallback: "color-mix(in oklab, #82aaff 65%, currentColor)",
        tags: &["f", "fb", "fc", "fd", "fm"],
    },
    Role {
        token: "syntax.type",
        fallback: "color-mix(in oklab, #ffcb6b 60%, currentColor)",
        tags: &["t", "tb", "td", "tq", "ns"],
    },
    Role {
        token: "syntax.variable",
        fallback: "currentColor",
        tags: &["v", "vb", "vm", "vp"],
    },
    Role {
        token: "syntax.property",
        fallback: "color-mix(in oklab, #82b1ff 65%, currentColor)",
        tags: &["pr", "prop"],
    },
    Role {
        token: "syntax.operator",
        fallback: "color-mix(in oklab, currentColor 70%, transparent)",
        tags: &["o"],
    },
    Role {
        token: "syntax.punctuation",
        fallback: "color-mix(in oklab, currentColor 70%, transparent)",
        tags: &["p", "pb", "pd", "ps"],
    },
    Role {
        token: "syntax.attribute",
        fallback: "color-mix(in oklab, #ffcb6b 60%, currentColor)",
        tags: &["a", "at"],
    },
    Role {
        token: "syntax.macro",
        fallback: "color-mix(in oklab, #c792ea 65%, currentColor)",
        tags: &["m", "pp"],
    },
    Role {
        token: "syntax.tag",
        fallback: "color-mix(in oklab, #ff5370 65%, currentColor)",
        tags: &["tg", "tag"],
    },
];

/// The tokens as `(name, default)`, for an app's `App` layer. Each
/// default is an expression over architect's tokens, so it follows the
/// palette showing.
pub const TOKENS: &[(&str, &str)] = &[
    ("syntax.keyword", "mix($ink, #c792ea, 0.65)"),
    ("syntax.string", "mix($ink, #c3e88d, 0.55)"),
    ("syntax.comment", "$ink-3"),
    ("syntax.number", "mix($ink, #f78c6c, 0.65)"),
    ("syntax.constant", "mix($ink, #ff5370, 0.65)"),
    ("syntax.function", "mix($ink, #82aaff, 0.65)"),
    ("syntax.type", "mix($ink, #ffcb6b, 0.60)"),
    ("syntax.variable", "$ink"),
    ("syntax.property", "mix($ink, #82b1ff, 0.65)"),
    ("syntax.operator", "$ink-2"),
    ("syntax.punctuation", "$ink-2"),
    ("syntax.attribute", "mix($ink, #ffcb6b, 0.60)"),
    ("syntax.macro", "mix($ink, #c792ea, 0.65)"),
    ("syntax.tag", "mix($ink, #ff5370, 0.65)"),
];

/// The theme token a scope tag is drawn in (`k` → `syntax.keyword`), or
/// none for a tag no role colours (markup text, errors).
#[must_use]
pub fn token_for_tag(tag: &str) -> Option<&'static str> {
    ROLES
        .iter()
        .find(|role| role.tags.contains(&tag))
        .map(|role| role.token)
}

/// A token's CSS variable (`syntax.keyword` → `--syntax-keyword`).
#[must_use]
pub fn css_var(token: &str) -> String {
    format!("--{}", token.replace('.', "-"))
}

/// The sheet that draws the editor's token classes in the theme's tokens.
///
/// Each class (`md-tok-k`, …) reads its `syntax.*` variable, falling back
/// to the editor's own colour where the app has not put the tokens in its
/// theme. Put it after the editor's sheet.
#[must_use]
pub fn css() -> String {
    let mut out = String::new();
    for role in ROLES {
        let selectors: Vec<String> = role
            .tags
            .iter()
            .map(|tag| format!(".editor-root .md-tok-{tag}"))
            .collect();
        out.push_str(&selectors.join(",\n"));
        out.push_str(" { color: var(");
        out.push_str(&css_var(role.token));
        out.push_str(", ");
        out.push_str(role.fallback);
        out.push_str("); }\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_token_list_is_the_roles() {
        let from_roles: Vec<&str> = ROLES.iter().map(|r| r.token).collect();
        let listed: Vec<&str> = TOKENS.iter().map(|(name, _)| *name).collect();
        assert_eq!(from_roles, listed);
        assert!(TOKENS.iter().all(|(name, _)| name.starts_with("syntax.")));
    }

    #[test]
    fn a_tag_has_one_role() {
        assert_eq!(token_for_tag("k"), Some("syntax.keyword"));
        assert_eq!(token_for_tag("s"), Some("syntax.string"));
        assert_eq!(token_for_tag("tx"), None);
        let mut seen = std::collections::BTreeSet::new();
        for role in ROLES {
            for tag in role.tags {
                assert!(seen.insert(*tag), "{tag} is in two roles");
            }
        }
    }

    #[test]
    fn the_sheet_reads_the_theme_variables() {
        let sheet = css();
        assert!(sheet.contains(".editor-root .md-tok-k"));
        assert!(sheet.contains("color: var(--syntax-keyword, "));
        assert_eq!(
            sheet
                .lines()
                .filter(|l| l.contains("var(--syntax-"))
                .count(),
            ROLES.len()
        );
    }
}
