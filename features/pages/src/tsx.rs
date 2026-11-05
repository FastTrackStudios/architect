//! TypeScript and JSX to JavaScript, in Rust (feature `tsx`).
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
    // The map's type is `oxc_sourcemap`'s, which this crate does not
    // name: its method is reached through a closure.
    #[allow(clippy::redundant_closure_for_method_calls)]
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

/// `text` (an error, a stack) with its mapped modules' places said as
/// their sources'.
///
/// Each `<module>:<line>:<column>` of a module in `maps` becomes its
/// source's (`board.tsx:12:5`), and other paths under `root` are made
/// relative to it. When its first line names no place, the first place
/// found leads it, so an error's first line says where it is.
#[must_use]
pub fn remap(text: &str, root: &str, maps: &[(&str, &SourceMap)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut first: Option<String> = None;
    let mut rest = text;
    while let Some((before, tail)) = rest.find(root).and_then(|at| rest.split_at_checked(at)) {
        out.push_str(before);
        // The module name that starts here, and what follows it.
        let end = tail
            .find(|c: char| c == ':' || c == ')' || c.is_whitespace())
            .unwrap_or(tail.len());
        let (module, after) = tail.split_at_checked(end).unwrap_or((tail, ""));
        let (place, used) = place_after(after);
        let shown = maps.iter().find(|(name, _)| *name == module).map_or_else(
            || {
                let rel = module
                    .strip_prefix(root)
                    .map_or(module, |r| r.trim_start_matches(['/', '\\']));
                at_place(rel, place)
            },
            |(_, map)| {
                let original = place.and_then(|(line, column)| map.original(line, column));
                at_place(&map.file, original)
            },
        );
        if place.is_some() && first.is_none() {
            first = Some(shown.clone());
        }
        out.push_str(&shown);
        rest = after.get(used..).unwrap_or_default();
    }
    out.push_str(rest);
    match first {
        Some(place) if !out.lines().next().is_some_and(|l| l.contains(&place)) => {
            format!("{place}: {out}")
        }
        _ => out,
    }
}

/// `file:line:column`, or `file` with no place.
fn at_place(file: &str, place: Option<(u32, u32)>) -> String {
    match place {
        Some((line, column)) => format!("{file}:{line}:{column}"),
        None => file.to_owned(),
    }
}

/// `:<line>:<column>` (or `:<line>`) at the start of `text`: the place,
/// and how many bytes of `text` it took.
fn place_after(text: &str) -> (Option<(u32, u32)>, usize) {
    /// The number at the start of `text`, and its length.
    fn number(text: &str) -> Option<(u32, usize)> {
        let len = text.bytes().take_while(u8::is_ascii_digit).count();
        let n = text.get(..len)?.parse().ok()?;
        Some((n, len))
    }
    let Some((line, line_len)) = text.strip_prefix(':').and_then(number) else {
        return (None, 0);
    };
    let taken = line_len.saturating_add(1);
    let column = text
        .get(taken..)
        .and_then(|t| t.strip_prefix(':'))
        .and_then(number);
    match column {
        Some((column, column_len)) => (
            Some((line, column)),
            taken.saturating_add(column_len).saturating_add(1),
        ),
        None => (Some((line, 1)), taken),
    }
}

/// oxc's `diagnostics` on `source` (named `file`), each at its place.
pub(crate) fn errors(
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
        let printed = code.lines().position(|l| l.contains("throw")).unwrap();
        let col = code.lines().nth(printed).unwrap().find("throw").unwrap();
        let line = u32::try_from(printed + 1).unwrap();
        let col = u32::try_from(col + 1).unwrap();
        assert_eq!(map.original(line, col), Some((6, 3)), "{code}");
        let text = format!("Error: boom 1\n    at f (/r/p.tsx:{line}:{col})\n");
        let said = remap(&text, "/r", &[("/r/p.tsx", &map)]);
        assert_eq!(
            said.lines().next(),
            Some("p.tsx:6:3: Error: boom 1"),
            "{said}"
        );
        assert!(said.contains("at f (p.tsx:6:3)"), "{said}");
        // A plain module's path is made relative, its place kept.
        let said = remap("x\n    at g (/r/job.js:4:2)", "/r", &[("/r/p.tsx", &map)]);
        assert!(said.starts_with("job.js:4:2: x"), "{said}");
    }

    #[test]
    fn a_column_counts_characters() {
        assert_eq!(line_col("ab\ncdé f", 9), (2, 6));
        assert_eq!(line_col("x", 0), (1, 1));
    }
}
