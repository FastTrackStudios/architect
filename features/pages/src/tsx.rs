//! TypeScript and JSX to JavaScript, in Rust (feature `js`): oxc parses
//! a `.ts`/`.tsx`/`.jsx` file, strips its types, turns its JSX into
//! `h(...)` calls (`Fragment` for `<>…</>`, both from `"architect"`), and
//! prints an ES module QuickJS runs. A page of a few hundred lines takes
//! a millisecond or two; a mistake is reported with its file, line and
//! column ([`SourceError`]).
//!
//! No type checking: that is the editor's, against the generated
//! `architect.d.ts` (`crate::codegen`). No bundling: imports stay
//! imports, resolved by the runtime's loader inside the page's folder.

use std::path::Path;

use oxc_allocator::Allocator;
use oxc_codegen::Codegen;
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
    Ok(Codegen::new().build(&program).code)
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
    fn a_column_counts_characters() {
        assert_eq!(line_col("ab\ncdé f", 9), (2, 6));
        assert_eq!(line_col("x", 0), (1, 1));
    }
}
