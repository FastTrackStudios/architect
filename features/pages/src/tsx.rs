//! TypeScript and JSX to JavaScript, in Rust (feature `js`).
//!
//! oxc parses a `.ts`/`.tsx`/`.jsx` file, strips its types, turns its JSX into
//! `h(...)` calls (`Fragment` for `<>…</>`, both from `"architect"`), and
//! prints an ES module `QuickJS` runs. A page of a few hundred lines takes
//! a millisecond or two; a mistake is reported with its file, line and
//! column ([`SourceError`]).
//!
//! It also keeps a source map ([`transpile_mapped`], [`SourceMap`]): what
//! the page throws as it runs is reported at the `.tsx` file's own line
//! and column, not the transpiled module's ([`remap`]).
//!
//! No type checking: that is the editor's, against the generated
//! `architect.d.ts` (`crate::codegen`). No bundling: imports stay
//! imports, resolved by the runtime's loader inside the page's folder.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{JsxOptions, JsxRuntime, TransformOptions, Transformer};

/// A mistake in a page's source, where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceError {
    /// The file, as it was named.
    pub file: String,
    /// 1-based; 0 when it has no place.
    pub line: u32,
    pub column: u32,
    pub message: String,
}

impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.line == 0 {
            write!(f, "{}: {}", self.file, self.message)
        } else {
            write!(
                f,
                "{}:{}:{}: {}",
                self.file, self.line, self.column, self.message
            )
        }
    }
}

impl std::error::Error for SourceError {}

/// Whether a file of this name is transpiled before it runs.
#[must_use]
pub fn needs_transpile(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("ts" | "tsx" | "jsx" | "mts")
    )
}

/// `source` (named `path`: its ending says TypeScript or JSX) as an ES
/// module of plain JavaScript.
///
/// # Errors
///
/// The first mistakes oxc found, each with its line and column.
pub fn transpile(path: &Path, source: &str) -> Result<String, Vec<SourceError>> {
    transpile_mapped(path, source).map(|(code, _)| code)
}

/// [`transpile`], with the map from the JavaScript it prints back to
/// `source`.
///
/// # Errors
///
/// As [`transpile`].
pub fn transpile_mapped(
    path: &Path,
    source: &str,
) -> Result<(String, SourceMap), Vec<SourceError>> {
    let file = path.display().to_string();
    let source_type = SourceType::from_path(path)
        .map_err(|e| vec![located(&file, source, None, e.to_string())])?
        .with_module(true);
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if !parsed.diagnostics.is_empty() {
        return Err(errors(&file, source, parsed.diagnostics));
    }
    let mut program = parsed.program;
    let semantic = SemanticBuilder::new().build(&program);
    if !semantic.diagnostics.is_empty() {
        return Err(errors(&file, source, semantic.diagnostics));
    }
    let scoping = semantic.semantic.into_scoping();
    let options = TransformOptions {
        jsx: JsxOptions {
            runtime: JsxRuntime::Classic,
            pragma: Some("h".to_owned()),
            pragma_frag: Some("Fragment".to_owned()),
            development: false,
            ..JsxOptions::enable()
        },
        ..TransformOptions::default()
    };
    let transformed =
        Transformer::new(&allocator, path, &options).build_with_scoping(scoping, &mut program);
    if !transformed.diagnostics.is_empty() {
        return Err(errors(&file, source, transformed.diagnostics));
    }
    let printed = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: Some(path.to_path_buf()),
            ..CodegenOptions::default()
        })
        .build(&program);
    let mut tokens: Vec<[u32; 4]> = printed
        .map
        .iter()
        .flat_map(|m| m.get_tokens())
        .map(|t| {
            [
                t.get_dst_line(),
                t.get_dst_col(),
                t.get_src_line(),
                t.get_src_col(),
            ]
        })
        .collect();
    tokens.sort_unstable();
    Ok((printed.code, SourceMap { file, tokens }))
}

/// Where each place in a transpiled module came from in its source: oxc's
/// map, kept as an owned table of `[line, column]` pairs (0-based, the
/// printed module's then the source's), sorted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceMap {
    /// The source's name, as it was given (relative to the page's folder).
    pub file: String,
    tokens: Vec<[u32; 4]>,
}

impl SourceMap {
    /// The source's line and column (1-based) for the printed module's
    /// `line` and `column` (1-based, as `QuickJS` reports them): the
    /// nearest mapped place at or before it on its line, else the line's
    /// first.
    #[must_use]
    pub fn original(&self, line: u32, column: u32) -> Option<(u32, u32)> {
        let (line, column) = (line.checked_sub(1)?, column.saturating_sub(1));
        let after = self
            .tokens
            .partition_point(|t| (t[0], t[1]) <= (line, column));
        let at = after
            .checked_sub(1)
            .and_then(|i| self.tokens.get(i))
            .filter(|t| t[0] == line)
            .or_else(|| self.tokens.get(after).filter(|t| t[0] == line))?;
        Some((at[2].saturating_add(1), at[3].saturating_add(1)))
    }
}

/// `text` (an error, a stack) with each `<module>:<line>:<column>` of a
/// mapped module said as its source's (`board.tsx:12:5`), and other
/// modules' paths of the page's folder made relative to `root`. When its
/// first line names no place, the first place found leads it, so an
/// error's first line says where it is.
#[must_use]
pub fn remap<'a>(
    text: &str,
    root: &str,
    maps: impl IntoIterator<Item = (&'a str, &'a SourceMap)> + Clone,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut first: Option<String> = None;
    let mut rest = text;
    while let Some(at) = rest.find(root) {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        // The longest module name that starts here.
        let module = tail
            .split(|c: char| c == ':' || c == ')' || c.is_whitespace())
            .next()
            .unwrap_or_default();
        let (place, used) = place_after(&tail[module.len()..]);
        let shown = maps
            .clone()
            .into_iter()
            .find(|(name, _)| *name == module)
            .map(|(_, map)| match place.and_then(|(l, c)| map.original(l, c)) {
                Some((l, c)) => format!("{}:{l}:{c}", map.file),
                None => map.file.clone(),
            })
            .unwrap_or_else(|| {
                let rel = module
                    .strip_prefix(root)
                    .map_or(module, |r| r.trim_start_matches(['/', '\\']));
                match place {
                    Some((l, c)) => format!("{rel}:{l}:{c}"),
                    None => rel.to_owned(),
                }
            });
        if place.is_some() && first.is_none() {
            first = Some(shown.clone());
        }
        out.push_str(&shown);
        rest = &tail[module.len().saturating_add(used)..];
    }
    out.push_str(rest);
    match first {
        Some(place) if !out.lines().next().is_some_and(|l| l.contains(&place)) => {
            format!("{place}: {out}")
        }
        _ => out,
    }
}

/// `:<line>:<column>` at the start of `s`: the place, and how much of `s`
/// it took.
fn place_after(s: &str) -> (Option<(u32, u32)>, usize) {
    let digits = |t: &str| t.bytes().take_while(u8::is_ascii_digit).count();
    let Some(t) = s.strip_prefix(':') else {
        return (None, 0);
    };
    let n = digits(t);
    let Ok(line) = t[..n].parse::<u32>() else {
        return (None, 0);
    };
    let t = &t[n..];
    let Some(c) = t.strip_prefix(':') else {
        return (Some((line, 1)), n.saturating_add(1));
    };
    let m = digits(c);
    match c[..m].parse::<u32>() {
        Ok(column) => (Some((line, column)), n.saturating_add(m).saturating_add(2)),
        Err(_) => (Some((line, 1)), n.saturating_add(1)),
    }
}

fn errors(
    file: &str,
    source: &str,
    diagnostics: impl IntoIterator<Item = oxc_diagnostics::OxcDiagnostic>,
) -> Vec<SourceError> {
    let mut out: Vec<SourceError> = diagnostics
        .into_iter()
        .map(|d| {
            let at = d
                .labels
                .iter()
                .find(|l| l.primary())
                .or_else(|| d.labels.first())
                .map(oxc_span::LabeledSpan::offset);
            let message = match &d.help {
                Some(help) => format!("{} ({help})", d.message),
                None => d.message.to_string(),
            };
            located(file, source, at, message)
        })
        .collect();
    if out.is_empty() {
        out.push(located(file, source, None, "it does not parse".to_owned()));
    }
    out
}

fn located(file: &str, source: &str, offset: Option<u32>, message: String) -> SourceError {
    let (line, column) = offset.map_or((0, 0), |o| line_col(source, o));
    SourceError {
        file: file.to_owned(),
        line,
        column,
        message,
    }
}

/// 1-based line and column (in characters) of byte `offset`.
fn line_col(source: &str, offset: u32) -> (u32, u32) {
    let at = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(source.len());
    let before = source.get(..at).unwrap_or(source);
    let line = before.matches('\n').count().saturating_add(1);
    let column = before
        .rsplit('\n')
        .next()
        .map_or(0, |l| l.chars().count())
        .saturating_add(1);
    (
        u32::try_from(line).unwrap_or(u32::MAX),
        u32::try_from(column).unwrap_or(u32::MAX),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn types_go_and_jsx_becomes_h() {
        let src = r#"import { h, Fragment, useState } from "architect";
type Pin = { title: string };
interface Props { pins: Pin[] }
export default function Board({ pins }: Props) {
  const [n, setN] = useState<number>(0);
  return <><div class="board" onClick={() => setN(n + 1)}>{pins.map((p) => <li key={p.title}>{p.title}</li>)}</div></>;
}
"#;
        let out = transpile(Path::new("board.tsx"), src).unwrap();
        assert!(out.contains("h(Fragment"), "{out}");
        assert!(out.contains("h(\"li\""), "{out}");
        assert!(!out.contains("interface"), "{out}");
        assert!(!out.contains(": Props"), "{out}");
    }

    #[test]
    fn a_mistake_says_where_it_is() {
        let src = "const a = 1;\nconst b = <div>;\n";
        let errs = transpile(Path::new("pages/x.tsx"), src).unwrap_err();
        let first = &errs[0];
        assert_eq!(first.file, "pages/x.tsx");
        assert_eq!(first.line, 2, "{first}");
        assert!(first.column > 0);
    }

    #[test]
    fn a_place_in_the_printed_module_is_its_sources() {
        // Types and an interface before the throw: the printed module's
        // line is not the source's.
        let src = "type A = { a: number };\ninterface B { b: string }\n\nexport function f(x: A) {\n  const y: number = x.a;\n  throw new Error(\"boom \" + y);\n}\n";
        let (code, map) = transpile_mapped(Path::new("p.tsx"), src).unwrap();
        let printed = code
            .lines()
            .position(|l| l.contains("throw"))
            .unwrap();
        let col = code.lines().nth(printed).unwrap().find("throw").unwrap();
        let line = u32::try_from(printed + 1).unwrap();
        let col = u32::try_from(col + 1).unwrap();
        assert_eq!(map.original(line, col), Some((6, 3)), "{code}");
        let text = format!("Error: boom 1\n    at f (/r/p.tsx:{line}:{col})\n");
        let said = remap(&text, "/r", [("/r/p.tsx", &map)]);
        assert_eq!(said.lines().next(), Some("p.tsx:6:3: Error: boom 1"), "{said}");
        assert!(said.contains("at f (p.tsx:6:3)"), "{said}");
        // A plain module's path is made relative, its place kept.
        let said = remap("x\n    at g (/r/job.js:4:2)", "/r", [("/r/p.tsx", &map)]);
        assert!(said.starts_with("job.js:4:2: x"), "{said}");
    }

    #[test]
    fn a_column_counts_characters() {
        assert_eq!(line_col("ab\ncdé f", 9), (2, 6));
        assert_eq!(line_col("x", 0), (1, 1));
    }
}
