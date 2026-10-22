//! The context a binding's `when` clause is checked against.
//!
//! The app describes where the user is (tags such as `panel:focused`,
//! variables such as `tab = performance`); a [`WhenExpr`] on a binding
//! layer decides from that whether the layer is live.

use std::collections::{HashMap, HashSet};

/// Tags (boolean flags) and variables (key/value pairs) describing where
/// input is going right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputContext {
    tags: HashSet<String>,
    vars: HashMap<String, String>,
}

/// A binding layer's condition, parsed from a tiny expression language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhenExpr {
    /// Always true.
    True,
    /// A tag is set.
    Tag(String),
    /// A variable has a value (`tab:performance`), or the tag
    /// `key:value` is set.
    VarEq { key: String, value: String },
    /// Every sub-expression is true.
    And(Vec<Self>),
}

impl InputContext {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a boolean tag (e.g. `panel:focused`).
    pub fn set_tag(&mut self, tag: impl Into<String>) {
        self.tags.insert(tag.into());
    }

    /// Remove a boolean tag.
    pub fn remove_tag(&mut self, tag: &str) {
        self.tags.remove(tag);
    }

    /// Whether a tag is set.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.contains(tag)
    }

    /// Set a variable (e.g. `mode` = `normal`).
    pub fn set_var(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.vars.insert(key.into(), value.into());
    }

    /// A variable's value.
    #[must_use]
    pub fn var(&self, key: &str) -> Option<&str> {
        self.vars.get(key).map(String::as_str)
    }

    /// Set the `mode` variable.
    pub fn set_mode(&mut self, mode: &str) {
        self.set_var("mode", mode);
    }

    /// Set the active tab: the `tab` variable, and the one `tab:<name>`
    /// tag (any other `tab:` tag is cleared).
    pub fn set_tab(&mut self, tab: &str) {
        self.tags.retain(|t| !t.starts_with("tab:"));
        self.set_var("tab", tab);
        self.set_tag(format!("tab:{tab}"));
    }
}

impl WhenExpr {
    /// Parse a when-clause:
    /// - empty or `true` is [`WhenExpr::True`];
    /// - `key:value` is a variable equality (or that tag);
    /// - `a && b:c` is a conjunction;
    /// - anything else is a tag.
    #[must_use]
    pub fn parse(input: &str) -> Self {
        let trimmed = input.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("true") {
            return Self::True;
        }
        let parts: Vec<&str> = trimmed
            .split("&&")
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        if parts.len() > 1 {
            return Self::And(parts.into_iter().map(Self::parse_atom).collect());
        }
        Self::parse_atom(trimmed)
    }

    fn parse_atom(input: &str) -> Self {
        match input.split_once(':') {
            Some((key, value)) => Self::VarEq {
                key: key.trim().to_owned(),
                value: value.trim().to_owned(),
            },
            None => Self::Tag(input.trim().to_owned()),
        }
    }

    /// Whether the expression holds in `ctx`.
    #[must_use]
    pub fn evaluate(&self, ctx: &InputContext) -> bool {
        match self {
            Self::True => true,
            Self::Tag(tag) => ctx.has_tag(tag),
            Self::VarEq { key, value } => {
                ctx.var(key).is_some_and(|v| v == value) || ctx.has_tag(&format!("{key}:{value}"))
            }
            Self::And(items) => items.iter().all(|expr| expr.evaluate(ctx)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_var_clause_matches_the_var_or_the_tag() {
        let expr = WhenExpr::parse("tab:performance");
        let mut ctx = InputContext::new();
        assert!(!expr.evaluate(&ctx));
        ctx.set_var("tab", "performance");
        assert!(expr.evaluate(&ctx));

        let mut tagged = InputContext::new();
        tagged.set_tag("tab:performance");
        assert!(expr.evaluate(&tagged));
    }

    #[test]
    fn a_conjunction_needs_every_part() {
        let expr = WhenExpr::parse("focused && mode:insert");
        let mut ctx = InputContext::new();
        ctx.set_tag("focused");
        assert!(!expr.evaluate(&ctx));
        ctx.set_mode("insert");
        assert!(expr.evaluate(&ctx));
    }

    #[test]
    fn set_tab_replaces_the_previous_tab_tag() {
        let mut ctx = InputContext::new();
        ctx.set_tab("a");
        ctx.set_tab("b");
        assert!(!ctx.has_tag("tab:a"));
        assert!(ctx.has_tag("tab:b"));
        assert_eq!(ctx.var("tab"), Some("b"));
    }

    #[test]
    fn empty_and_true_are_always() {
        assert_eq!(WhenExpr::parse(""), WhenExpr::True);
        assert_eq!(WhenExpr::parse(" TRUE "), WhenExpr::True);
    }
}
