//! Token values that name other tokens.
//!
//! `$border`, `mix($border, $foreground, 0.2)`, `alpha($ink, 0.3)`, nested
//! as deep as wanted. An app ships its defaults as these, so they follow
//! whatever palette the person picks.
//!
//! ```text
//! value := expr | literal               a literal is any other CSS value, kept as is
//! expr  := '$' name
//!        | 'mix' '(' arg ',' arg ',' arg ')'    two colours and t, 0 to 1 (OKLab)
//!        | 'alpha' '(' arg ',' arg ')'         a colour with alpha t, 0 to 1
//! arg   := expr | literal               a colour (`#fff`, `rgb(…)`) or a number (`0.2`, `20%`)
//! name  := segment ('.' segment)*       segment := [a-z0-9-]+
//! ```
//!
//! A value is an expression only when the whole of it is one: it starts
//! with `$`, `mix(` or `alpha(`. Anything else (`0.625rem`,
//! `0 6px 20px rgba(0,0,0,0.35)`) is a literal and never parsed.
//! [`crate::resolve`] evaluates them after derivation, against the final
//! value of every name (`crate::resolve::Resolved::source` says which ones
//! were expressions).

use crate::model::valid_token;

/// A parsed expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    /// `$name`: the final value of token `name`.
    Ref(String),
    /// `mix(a, b, t)`: `a` and `b` mixed in `OKLab`, `t` 0 (`a`) to 1 (`b`).
    Mix(Box<Self>, Box<Self>, Box<Self>),
    /// `alpha(a, t)`: colour `a` with alpha `t`.
    Alpha(Box<Self>, Box<Self>),
    /// A colour or a number, as written.
    Lit(String),
}

impl Expr {
    /// Every token it names, once each, in the order written.
    #[must_use]
    pub fn references(&self) -> Vec<&str> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a str>) {
        match self {
            Self::Ref(name) => {
                if !out.contains(&name.as_str()) {
                    out.push(name);
                }
            }
            Self::Mix(a, b, t) => {
                a.collect(out);
                b.collect(out);
                t.collect(out);
            }
            Self::Alpha(a, t) => {
                a.collect(out);
                t.collect(out);
            }
            Self::Lit(_) => {}
        }
    }
}

/// Whether `value` is meant as an expression (it starts with `$`, `mix(`
/// or `alpha(`); a literal otherwise.
#[must_use]
pub fn is_expression(value: &str) -> bool {
    let v = value.trim_start();
    v.starts_with('$') || call_name(v).is_some()
}

/// `value` parsed: `Ok(None)` for a literal, `Err` with what is wrong for
/// an expression that does not read.
pub fn parse(value: &str) -> Result<Option<Expr>, String> {
    if is_expression(value) {
        parse_expr(value).map(Some)
    } else {
        Ok(None)
    }
}

/// `mix` or `alpha` when `v` starts as a call to one.
fn call_name(v: &str) -> Option<&'static str> {
    ["mix", "alpha"].into_iter().find(|name| {
        v.strip_prefix(name)
            .is_some_and(|rest| rest.trim_start().starts_with('('))
    })
}

fn parse_expr(text: &str) -> Result<Expr, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("an empty argument".to_owned());
    }
    if let Some(name) = text.strip_prefix('$') {
        return if valid_token(name) {
            Ok(Expr::Ref(name.to_owned()))
        } else {
            Err(format!(
                "`{text}`: a reference is `$` and a token name (`$border`, `$arrange.item.edge`)"
            ))
        };
    }
    let Some(name) = call_name(text) else {
        return Ok(Expr::Lit(text.to_owned()));
    };
    let args = call_args(text, name)?;
    let parsed = args
        .iter()
        .map(|a| parse_expr(a))
        .collect::<Result<Vec<_>, _>>()?;
    if name == "mix" {
        let [a, b, t]: [Expr; 3] = parsed.try_into().map_err(|p: Vec<Expr>| {
            format!(
                "`{text}`: mix takes three arguments (a colour, a colour, t from 0 to 1), not {}",
                p.len()
            )
        })?;
        Ok(Expr::Mix(Box::new(a), Box::new(b), Box::new(t)))
    } else {
        let [a, t]: [Expr; 2] = parsed.try_into().map_err(|p: Vec<Expr>| {
            format!(
                "`{text}`: alpha takes two arguments (a colour, an alpha from 0 to 1), not {}",
                p.len()
            )
        })?;
        Ok(Expr::Alpha(Box::new(a), Box::new(t)))
    }
}

/// The arguments of `name(…)`, split at the commas outside any brackets.
fn call_args<'a>(text: &'a str, name: &str) -> Result<Vec<&'a str>, String> {
    let open = text
        .find('(')
        .ok_or_else(|| format!("`{text}`: {name} needs `(`"))?;
    let mut depth = 0usize;
    let mut close = None;
    for (i, c) in text.char_indices().skip_while(|(i, _)| *i < open) {
        match c {
            '(' => depth = depth.saturating_add(1),
            ')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    close = Some(i);
                    break;
                }
            }
            _ => {}
        }
    }
    let close = close.ok_or_else(|| format!("`{text}`: a `(` without its `)`"))?;
    let after = text.get(close.saturating_add(1)..).unwrap_or_default();
    if !after.trim().is_empty() {
        return Err(format!("`{text}`: {after:?} after the `)`"));
    }
    let inner = text.get(open.saturating_add(1)..close).unwrap_or_default();
    let mut args = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    for (i, c) in inner.char_indices() {
        match c {
            '(' => depth = depth.saturating_add(1),
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                args.push(inner.get(start..i).unwrap_or_default());
                start = i.saturating_add(1);
            }
            _ => {}
        }
    }
    args.push(inner.get(start..).unwrap_or_default());
    Ok(args)
}

/// A number as an expression's argument: `0.2` or `20%`.
pub(crate) fn number(text: &str) -> Option<f32> {
    let text = text.trim();
    let n = match text.strip_suffix('%') {
        Some(pct) => pct.trim().parse::<f32>().ok()? / 100.0,
        None => text.parse::<f32>().ok()?,
    };
    n.is_finite().then_some(n)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unnecessary_box_returns, clippy::unwrap_used)]

    use super::*;

    fn r(name: &str) -> Box<Expr> {
        Box::new(Expr::Ref(name.to_owned()))
    }

    fn lit(text: &str) -> Box<Expr> {
        Box::new(Expr::Lit(text.to_owned()))
    }

    #[test]
    fn literals_are_not_parsed() {
        for v in [
            "#fff",
            "0.625rem",
            "0 6px 20px rgba(0,0,0,0.35)",
            "12",
            "mixed",
        ] {
            assert_eq!(parse(v), Ok(None), "{v}");
        }
    }

    #[test]
    fn references_and_calls_nest() {
        assert_eq!(parse("$border"), Ok(Some(Expr::Ref("border".into()))));
        assert_eq!(
            parse(" mix($border, $foreground, 0.2) "),
            Ok(Some(Expr::Mix(r("border"), r("foreground"), lit("0.2"))))
        );
        let nested = parse("mix(alpha($arrange.item.edge, 0.3), rgb(1, 2, 3), $t)")
            .unwrap()
            .unwrap();
        assert_eq!(
            nested,
            Expr::Mix(
                Box::new(Expr::Alpha(r("arrange.item.edge"), lit("0.3"))),
                lit("rgb(1, 2, 3)"),
                r("t"),
            )
        );
        assert_eq!(nested.references(), ["arrange.item.edge", "t"]);
    }

    #[test]
    fn a_bad_expression_says_what_is_wrong() {
        let err = |v: &str| parse(v).unwrap_err();
        assert!(
            err("mix($a, $b)").contains("three arguments"),
            "{}",
            err("mix($a, $b)")
        );
        assert!(err("alpha($a)").contains("two arguments"));
        assert!(err("mix($a, $b, 0.5").contains("without its `)`"));
        assert!(err("alpha($a, 0.5) extra").contains("after the `)`"));
        assert!(err("$Bad").contains("a reference is"));
        assert!(err("$").contains("a reference is"));
        assert!(err("alpha(, 0.5)").contains("empty argument"));
    }

    #[test]
    fn numbers_take_a_percent() {
        assert_eq!(number("0.25"), Some(0.25));
        assert_eq!(number("20%"), Some(0.2));
        assert_eq!(number("a"), None);
        assert_eq!(number("NaN"), None);
    }
}
