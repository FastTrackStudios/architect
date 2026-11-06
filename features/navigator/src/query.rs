//! The search a ⌘/Ctrl-click builds: tags and properties joined by AND
//! and OR, as the filter search writes them.
//!
//! Notebook Navigator's tag-mode query (`src/utils/filterSearch.ts`
//! `updateFilterQueryWithTag`, `updateFilterQueryWithProperty`,
//! `appendMutationToken`, `removeMutationToken`,
//! `formatPropertyFilterPartForQuery`; `src/utils/filterSearchExpression.ts`
//! `evaluateTagExpression`):
//!
//! - A ⌘/Ctrl-click on a tag or a property adds it with AND, with ⇧ too
//!   with OR; on one already there it takes it out (and the AND or OR
//!   before it).
//! - Written as `#tag`, `.key` and `.key=value`, joined by `AND` and `OR`;
//!   a key or value with a space, `"`, `\` or `=` in quotes, those escaped
//!   (`\"`, `\\`, `\=`).
//! - AND binds tighter than OR, no brackets: `#a OR #b AND #c` is `#a`, or
//!   `#b` and `#c`.
//! - A tag matches a note with it or a tag below it (any case); `.key` a
//!   note with a key that starts with it; `.key=value` a note with that
//!   key whose value holds `value` (any case).
//!
//! The search pane (#338) reads and writes the same [`Query`]: its text is
//! [`Query::text`], and [`Query::parse`] reads a typed one back when it is
//! made of tags and properties alone.

use crate::properties::{fold, value_display, values_of};
use crate::source::NoteFile;
use crate::tags::{is_or_below, tag_key};

/// How an operand joins the one before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    And,
    Or,
}

/// A tag or a property in the search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// A tag's key.
    Tag(String),
    /// A key (folded), and a value (folded) or none for any.
    Property { key: String, value: Option<String> },
}

impl Operand {
    /// A tag, keyed.
    #[must_use]
    pub fn tag(tag: &str) -> Self {
        Self::Tag(tag_key(tag))
    }

    /// A property: `key`, or `key` with `value` (as it shows).
    #[must_use]
    pub fn property(key: &str, value: Option<&str>) -> Self {
        Self::Property {
            key: fold(key),
            value: value.map(|v| fold(&value_display(v))),
        }
    }

    /// Whether `f` matches it.
    #[must_use]
    pub fn matches(&self, f: &NoteFile) -> bool {
        match self {
            Self::Tag(want) => f.tags.iter().any(|t| is_or_below(&tag_key(t), want)),
            Self::Property { key, value: None } => f
                .properties
                .iter()
                .any(|(k, _)| fold(k).starts_with(key.as_str())),
            Self::Property {
                key,
                value: Some(want),
            } => values_of(f, key).is_some_and(|(parts, _, _)| {
                parts
                    .iter()
                    .any(|p| fold(&value_display(p)).contains(want.as_str()))
            }),
        }
    }

    /// As the search writes it.
    #[must_use]
    pub fn text(&self) -> String {
        match self {
            Self::Tag(t) => format!("#{t}"),
            Self::Property { key, value: None } => format!(".{}", quoted(key)),
            Self::Property {
                key,
                value: Some(v),
            } => format!(".{}={}", quoted(key), quoted(v)),
        }
    }
}

/// A key or value as the search writes it.
fn quoted(part: &str) -> String {
    let needs = part
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '"' | '\\' | '='));
    if needs {
        let inner = part
            .replace('\\', "\\\\")
            .replace('=', "\\=")
            .replace('"', "\\\"");
        format!("\"{inner}\"")
    } else {
        part.to_owned()
    }
}

/// Tags and properties, each but the first joined to the one before.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Query {
    pub terms: Vec<(Join, Operand)>,
}

impl Query {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Takes `operand` out when it is there, else adds it joined by `join`
    /// (a ⌘/Ctrl-click, `join` OR with ⇧).
    pub fn toggle(&mut self, operand: Operand, join: Join) {
        if let Some(at) = self.terms.iter().position(|(_, o)| *o == operand) {
            self.terms.remove(at);
        } else {
            self.terms.push((join, operand));
        }
    }

    /// Whether `f` matches: OR between runs of AND.
    #[must_use]
    pub fn matches(&self, f: &NoteFile) -> bool {
        if self.terms.is_empty() {
            return true;
        }
        let mut any = false;
        let mut run = true;
        for (i, (join, o)) in self.terms.iter().enumerate() {
            if i > 0 && *join == Join::Or {
                any |= run;
                run = true;
            }
            run &= o.matches(f);
        }
        any || run
    }

    /// As the search writes it: `#a AND .status=done OR #b`.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = String::new();
        for (i, (join, o)) in self.terms.iter().enumerate() {
            if i > 0 {
                out.push_str(match join {
                    Join::And => " AND ",
                    Join::Or => " OR ",
                });
            }
            out.push_str(&o.text());
        }
        out
    }

    /// A search made of tags and properties alone, read back; `None` for
    /// any other (it is the filter search's, #338).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut terms = Vec::new();
        let mut join = Join::And;
        for word in words(text) {
            match word.to_lowercase().as_str() {
                "and" => join = Join::And,
                "or" => join = Join::Or,
                _ => {
                    let operand = if let Some(t) = word.strip_prefix('#') {
                        (!t.is_empty()).then(|| Operand::tag(t))?
                    } else {
                        let (k, v) = split_eq(word.strip_prefix('.')?);
                        Operand::Property {
                            key: fold(&unquote(&k)),
                            value: v.map(|v| fold(&unquote(&v))),
                        }
                    };
                    terms.push((join, operand));
                    join = Join::And;
                }
            }
        }
        Some(Self { terms })
    }
}

/// `text` split on spaces outside quotes.
fn words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in text.chars() {
        if escaped {
            cur.push(c);
            escaped = false;
        } else if c == '\\' {
            cur.push(c);
            escaped = true;
        } else if c == '"' {
            cur.push(c);
            quoted = !quoted;
        } else if c.is_whitespace() && !quoted {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `key=value` at the first `=` not escaped or quoted.
fn split_eq(p: &str) -> (String, Option<String>) {
    let (mut quoted, mut escaped) = (false, false);
    for (i, c) in p.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => quoted = !quoted,
            '=' if !quoted => {
                return (
                    p.get(..i).unwrap_or_default().to_owned(),
                    Some(p.get(i.saturating_add(1)..).unwrap_or_default().to_owned()),
                );
            }
            _ => {}
        }
    }
    (p.to_owned(), None)
}

/// A part without its quotes and escapes.
fn unquote(part: &str) -> String {
    let inner = part
        .strip_prefix('"')
        .and_then(|p| p.strip_suffix('"'))
        .unwrap_or(part);
    let mut out = String::new();
    let mut escaped = false;
    for c in inner.chars() {
        if escaped || c != '\\' {
            out.push(c);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    out
}
