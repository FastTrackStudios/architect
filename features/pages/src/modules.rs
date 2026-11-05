//! A page's folder as modules, for a runtime that loads them by URL
//! (feature `tsx`): the web runner's (`crate::web`), whose Web Worker
//! imports each module from a `blob:` URL of its own.
//!
//! [`prepare`] does here what the `QuickJS` loader (`crate::js`) does as
//! each module loads: every script of the folder is transpiled (oxc,
//! [`crate::tsx`]) and its imports are resolved by the same rules —
//! `"architect"` (`"architect/worker"` in a worker's script) and files of
//! the page's own folder (`./list.tsx`, `../shared/ui.ts`), nothing else.
//! Each resolved import's specifier is rewritten to [`LINK`] and the
//! module's name, which the worker swaps for that module's URL as it
//! links them. A mistake (a file that does not parse, an import that does
//! not resolve) stays with its module: it fails the page only when that
//! module is imported, as natively.
//!
//! Only static imports and re-exports are resolved: a dynamic `import()`
//! is not, on the web.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::ModuleDeclaration;
use oxc_parser::Parser;
use oxc_span::SourceType;

use crate::tsx;

/// `import … from "architect"`: a page's runtime (the prelude's
/// `__architect.api`).
pub const ARCHITECT: &str = "const a = globalThis.__architect.api;\n\
export const { h, Fragment, useState, useRef, useEffect, useData, useShared, useEmbed, useTheme, invalidate, \
theme, actions, settings, state, notify, ui, clipboard, events, documents, editor, net, ports, worker, Overlay, host } = a;\nexport default a;\n";

/// `import … from "architect/worker"`: a worker's.
pub const ARCHITECT_WORKER: &str = "const w = globalThis.__worker.api;\n\
export const { args, post, onMessage } = w;\nexport default w;\n";

/// What a resolved import's specifier becomes: this, then the module's
/// name (`architect-module:list.tsx`).
pub const LINK: &str = "architect-module:";

/// One script of a page's folder.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Module {
    /// Its path in the folder, `/`-separated (`board.tsx`, `ui/list.ts`).
    pub name: String,
    /// It as JavaScript, its imports rewritten to [`LINK`]s.
    pub code: String,
    /// What it imports: each specifier as written, and the module it
    /// names (`architect`, `architect/worker` or a file's name), or why
    /// it names none.
    pub imports: Vec<Import>,
    /// Why it cannot run (it does not parse), with its line and column.
    pub error: Option<String>,
    /// From its JavaScript back to its source, when it was transpiled.
    pub map: Option<tsx::SourceMap>,
}

/// One import of a [`Module`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub specifier: String,
    pub to: Result<String, String>,
}

/// Whether a file of this name is a script a page may import or spawn.
#[must_use]
pub fn is_script(name: &str) -> bool {
    matches!(
        Path::new(name).extension().and_then(|e| e.to_str()),
        Some("ts" | "tsx" | "jsx" | "mts" | "js" | "mjs")
    )
}

/// The folder's scripts as modules, by name; its other files are left
/// out. `files` is the folder's every file, by its `/`-separated path.
#[must_use]
pub fn prepare(files: &BTreeMap<String, String>) -> BTreeMap<String, Module> {
    files
        .iter()
        .filter(|(name, _)| is_script(name))
        .map(|(name, source)| (name.clone(), module(name, source, files)))
        .collect()
}

fn module(name: &str, source: &str, files: &BTreeMap<String, String>) -> Module {
    let path = Path::new(name);
    let (code, map) = if tsx::needs_transpile(path) {
        match tsx::transpile_mapped(path, source) {
            Ok((code, map)) => (code, Some(map)),
            Err(errors) => {
                return Module {
                    name: name.to_owned(),
                    error: Some(errors.first().map(ToString::to_string).unwrap_or_default()),
                    ..Module::default()
                };
            }
        }
    } else {
        (source.to_owned(), None)
    };
    let found = match specifiers(name, &code) {
        Ok(found) => found,
        Err(why) => {
            return Module {
                name: name.to_owned(),
                error: Some(why),
                ..Module::default()
            };
        }
    };
    let mut code = code;
    let mut imports = Vec::new();
    // From the end, so the earlier places stay where they are.
    for (start, end, specifier) in found.into_iter().rev() {
        let to = resolve(name, &specifier, files);
        if let Ok(target) = &to {
            let link = serde_json::Value::String(format!("{LINK}{target}")).to_string();
            if let Some(range) = code.get(start..end).map(|_| start..end) {
                code.replace_range(range, &link);
            }
        }
        imports.push(Import { specifier, to });
    }
    imports.reverse();
    Module {
        name: name.to_owned(),
        code,
        imports,
        error: None,
        map,
    }
}

/// Each static import's and re-export's specifier in `code` (a module of
/// plain JavaScript): its string literal's byte range, and its value.
fn specifiers(name: &str, code: &str) -> Result<Vec<(usize, usize, String)>, String> {
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, code, SourceType::mjs()).parse();
    if !parsed.diagnostics.is_empty() {
        let errors = tsx::errors(name, code, parsed.diagnostics);
        return Err(errors.first().map(ToString::to_string).unwrap_or_default());
    }
    let mut out = Vec::new();
    for statement in &parsed.program.body {
        let Some(declaration) = statement.as_module_declaration() else {
            continue;
        };
        let source = match declaration {
            ModuleDeclaration::ImportDeclaration(d) => Some(&d.source),
            ModuleDeclaration::ExportAllDeclaration(d) => Some(&d.source),
            ModuleDeclaration::ExportFromDeclaration(d) => Some(&d.source),
            _ => None,
        };
        if let Some(s) = source {
            let (start, end) = (s.span.start, s.span.end);
            out.push((
                usize::try_from(start).unwrap_or(usize::MAX),
                usize::try_from(end).unwrap_or(usize::MAX),
                s.value.display().to_string(),
            ));
        }
    }
    Ok(out)
}

/// What `specifier`, imported by module `from`, names: `architect`,
/// `architect/worker`, or a file of the folder — the `QuickJS` loader's
/// rules (`crate::js`), over the folder's names.
///
/// # Errors
///
/// It is a package or a URL, it leaves the folder, or no file has it.
pub fn resolve(
    from: &str,
    specifier: &str,
    files: &BTreeMap<String, String>,
) -> Result<String, String> {
    if specifier == "architect" || specifier == "architect/worker" {
        return Ok(specifier.to_owned());
    }
    if !(specifier.starts_with("./") || specifier.starts_with("../")) {
        return Err(
            "a page imports \"architect\" and files of its own folder (\"./x.tsx\"), nothing else"
                .to_owned(),
        );
    }
    let base = Path::new(from).parent().unwrap_or_else(|| Path::new(""));
    let wanted = inside(&base.join(specifier)).ok_or("outside the page's folder")?;
    let candidates = [
        wanted.clone(),
        wanted.with_extension("tsx"),
        wanted.with_extension("ts"),
        wanted.with_extension("jsx"),
        wanted.with_extension("js"),
        wanted.join("index.tsx"),
        wanted.join("index.ts"),
        wanted.join("index.js"),
    ];
    candidates
        .iter()
        .map(|c| slashed(c))
        .find(|c| files.contains_key(c))
        .ok_or_else(|| "no such file in the page's folder".to_owned())
}

/// `a/./b/../c` as `a/c`; `None` when it climbs out of the folder.
fn inside(path: &Path) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            Component::Normal(p) => out.push(p),
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

fn slashed(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(p) => Some(p.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use super::*;

    fn folder(files: &[(&str, &str)]) -> BTreeMap<String, String> {
        files
            .iter()
            .map(|(n, s)| ((*n).to_owned(), (*s).to_owned()))
            .collect()
    }

    #[test]
    fn imports_resolve_in_the_folder_and_become_links() {
        let files = folder(&[
            (
                "board.tsx",
                "import { h } from \"architect\";\nimport { Row } from \"./ui/row\";\nexport default () => <Row />;\n",
            ),
            (
                "ui/row.tsx",
                "import { h } from \"architect\";\nexport * from \"../shared.js\";\nexport const Row = () => <div />;\n",
            ),
            ("shared.js", "export const x = 1;\n"),
            ("job.js", "import { post } from \"architect/worker\";\n"),
            ("count.luau", "print(1)"),
        ]);
        let modules = prepare(&files);
        assert_eq!(
            modules.keys().collect::<Vec<_>>(),
            ["board.tsx", "job.js", "shared.js", "ui/row.tsx"]
        );
        let board = &modules["board.tsx"];
        assert_eq!(board.error, None);
        assert!(
            board.code.contains("from \"architect-module:architect\""),
            "{}",
            board.code
        );
        assert!(
            board.code.contains("from \"architect-module:ui/row.tsx\""),
            "{}",
            board.code
        );
        assert_eq!(board.imports[1].to, Ok("ui/row.tsx".to_owned()));
        let row = &modules["ui/row.tsx"];
        assert!(
            row.code
                .contains("export * from \"architect-module:shared.js\""),
            "{}",
            row.code
        );
        assert_eq!(
            modules["job.js"].imports[0].to,
            Ok("architect/worker".to_owned())
        );
    }

    #[test]
    fn nothing_outside_the_folder_and_no_packages() {
        let files = folder(&[(
            "p.js",
            "import a from \"../up.js\";\nimport b from \"react\";\nimport c from \"./gone\";\n",
        )]);
        let p = &prepare(&files)["p.js"];
        assert_eq!(p.imports[0].to, Err("outside the page's folder".to_owned()));
        assert!(
            p.imports[1]
                .to
                .as_ref()
                .unwrap_err()
                .contains("nothing else")
        );
        assert_eq!(
            p.imports[2].to,
            Err("no such file in the page's folder".to_owned())
        );
    }

    #[test]
    fn a_broken_file_keeps_its_place() {
        let files = folder(&[
            ("x.tsx", "const a = 1;\nconst b = <div>;\n"),
            ("y.js", "const a = 1;\nconst = 2;\n"),
        ]);
        let modules = prepare(&files);
        let x = &modules["x.tsx"];
        assert!(x.error.as_deref().unwrap().starts_with("x.tsx:2:"), "{x:?}");
        let y = &modules["y.js"];
        assert!(y.error.as_deref().unwrap().starts_with("y.js:2:"), "{y:?}");
    }
}
