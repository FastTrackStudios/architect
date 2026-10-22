//! `cargo xtask docs`: every path or command the docs name must exist.
//!
//! The docs once described an `examples/app/`, an `architect-cli` and
//! `cargo xtask idioms` years after none of them existed, and nothing
//! noticed. This check reads `docs/content/**/*.md` and `README.md` and
//! fails on any of these that does not resolve in the working tree:
//!
//! - a repo path in a code span or a fenced block (`crates/architect`,
//!   `examples/greeter/src/lib.rs:22-40`): a token whose first segment is
//!   a top-level directory of the repo, line suffixes stripped;
//! - a page link (`@/architecture/peers.md`) or a source link into this
//!   repo's `main` (`…/src/branch/main/<path>`, `…/blob/main/<path>`);
//! - `just <recipe>` against the `justfile`;
//! - `cargo xtask <command>` / `cargo run -p xtask -- <command>` against
//!   this binary's own subcommands;
//! - `cargo … -p <package>` against the packages under the root.
//!
//! Not checked, on purpose: tokens with a placeholder (`<f>`, `*`, `…`,
//! `$X`), paths in other repositories written with a repo prefix
//! (`task:apps/web`, `studio:features/session`), and anything between
//! `<!-- doc-check: off … -->` and `<!-- doc-check: on -->` (sketches of
//! layouts that are not this repo's).

use std::collections::BTreeSet;
use std::fmt;
use std::path::Path;

/// What the docs may name: everything that exists.
#[derive(Debug, Default)]
pub struct Facts {
    /// Every file and directory under the root, `/`-separated, relative.
    pub paths: BTreeSet<String>,
    /// The root's directories: a token is a path only if it starts with one.
    pub top_dirs: BTreeSet<String>,
    /// `justfile` recipe names.
    pub recipes: BTreeSet<String>,
    /// `cargo xtask` subcommand names.
    pub xtask: BTreeSet<String>,
    /// Package names of every `Cargo.toml` under the root.
    pub packages: BTreeSet<String>,
}

/// One name that does not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub file: String,
    pub line: usize,
    pub kind: Kind,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Path,
    Page,
    Recipe,
    Xtask,
    Package,
}

impl fmt::Display for Finding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            Kind::Path => "path",
            Kind::Page => "page",
            Kind::Recipe => "just recipe",
            Kind::Xtask => "cargo xtask command",
            Kind::Package => "package",
        };
        write!(
            f,
            "{}:{}: {what} `{}` does not exist",
            self.file, self.line, self.name
        )
    }
}

const OFF: &str = "<!-- doc-check: off";
const ON: &str = "<!-- doc-check: on";
const PLACEHOLDER: &[char] = &['<', '>', '*', '{', '}', '…', '$', '|'];
const SOURCE_LINKS: &[&str] = &["/src/branch/main/", "/blob/main/", "/tree/main/"];
const REPO: &str = "architect";

/// Every finding in one markdown file's `text`.
pub fn check_text(file: &str, text: &str, facts: &Facts) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut fenced = false;
    let mut off = false;
    // A code span can wrap onto the next line; a blank line ends it.
    let mut in_span = false;
    for (line, raw) in (1_usize..).zip(text.lines()) {
        if raw.contains(OFF) {
            off = true;
            continue;
        }
        if raw.contains(ON) {
            off = false;
            continue;
        }
        if raw.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if raw.trim().is_empty() {
            in_span = false;
        }
        if off {
            continue;
        }
        let mut found = |kind: Kind, name: &str| {
            out.push(Finding {
                file: file.to_owned(),
                line,
                kind,
                name: name.to_owned(),
            });
        };
        let spans: Vec<&str> = if fenced {
            vec![raw]
        } else {
            let start = in_span;
            in_span ^= !raw.matches('`').count().is_multiple_of(2);
            raw.split('`')
                .zip([start, !start].into_iter().cycle())
                .filter_map(|(part, code)| code.then_some(part))
                .collect()
        };
        for span in &spans {
            for token in span.split(is_separator) {
                if let Some(path) = repo_path(token, facts)
                    && !facts.paths.contains(path)
                {
                    found(Kind::Path, path);
                }
            }
            for (kind, name) in commands(span) {
                let known = match kind {
                    Kind::Recipe => facts.recipes.contains(name),
                    Kind::Xtask => facts.xtask.contains(name),
                    Kind::Package => facts.packages.contains(name),
                    Kind::Path | Kind::Page => true,
                };
                if !known {
                    found(kind, name);
                }
            }
        }
        if !fenced {
            for target in link_targets(raw) {
                if let Some(page) = target.strip_prefix("@/") {
                    let page = page.split('#').next().unwrap_or(page);
                    let path = format!("docs/content/{page}");
                    if !facts.paths.contains(&path) {
                        found(Kind::Page, &path);
                    }
                    continue;
                }
                if let Some(path) = source_link(target)
                    && !facts.paths.contains(path)
                {
                    found(Kind::Path, path);
                }
            }
        }
    }
    out
}

const fn is_separator(c: char) -> bool {
    matches!(
        c,
        ' ' | '\t' | '(' | ')' | ',' | ';' | '"' | '\'' | '=' | '[' | ']'
    )
}

/// `token` as a repo path to check, or `None` when it is not one.
fn repo_path<'a>(token: &'a str, facts: &Facts) -> Option<&'a str> {
    let token = token.trim_end_matches(['.', ',', ':']);
    let path = strip_line_suffix(token).trim_end_matches(['.', ':', ',', '/']);
    let (first, rest) = path.split_once('/')?;
    if rest.is_empty()
        || !facts.top_dirs.contains(first)
        || path.contains(PLACEHOLDER)
        || path.contains("..")
    {
        return None;
    }
    Some(path)
}

/// `a/b.rs:12-40,55` → `a/b.rs`.
fn strip_line_suffix(token: &str) -> &str {
    let mut path = token;
    while let Some((head, tail)) = path.rsplit_once(':') {
        let is_lines =
            !tail.is_empty() && tail.chars().all(|c| c.is_ascii_digit() || c == '-' || c == ',');
        if !is_lines {
            break;
        }
        path = head;
    }
    path
}

/// The recipes, xtask commands and packages a command line names.
fn commands(span: &str) -> Vec<(Kind, &str)> {
    let mut out = Vec::new();
    for command in span.split(['&', ';', '|']) {
        let words: Vec<&str> = command.split_whitespace().collect();
        let words = match words.split_first() {
            Some((&"$", rest)) => rest,
            _ => words.as_slice(),
        };
        match words {
            ["just", recipe, ..] if is_name(recipe) => out.push((Kind::Recipe, *recipe)),
            ["cargo", "xtask", name, ..] if is_name(name) => out.push((Kind::Xtask, *name)),
            ["cargo", rest @ ..] => {
                let mut prev = "";
                let mut xtask_run = false;
                for (i, word) in rest.iter().enumerate() {
                    if (prev == "-p" || prev == "--package") && is_name(word) {
                        if *word == "xtask" {
                            xtask_run = true;
                        } else {
                            out.push((Kind::Package, *word));
                        }
                    }
                    if xtask_run && *word == "--" {
                        if let Some(name) = rest.get(i.saturating_add(1))
                            && is_name(name)
                        {
                            out.push((Kind::Xtask, *name));
                        }
                        break;
                    }
                    prev = *word;
                }
            }
            _ => {}
        }
    }
    out
}

fn is_name(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_alphabetic())
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The `target` of every `[text](target)` on a line.
fn link_targets(line: &str) -> Vec<&str> {
    line.split("](")
        .skip(1)
        .filter_map(|rest| rest.split([')', ' ']).next())
        .collect()
}

/// The repo path a source link into this repo's `main` points at.
fn source_link(url: &str) -> Option<&str> {
    if !url.starts_with("http") || !url.contains(&format!("/{REPO}/")) {
        return None;
    }
    SOURCE_LINKS.iter().find_map(|marker| {
        let (_, path) = url.split_once(marker)?;
        let path = path.split('#').next().unwrap_or(path).trim_end_matches('/');
        (!path.is_empty()).then_some(path)
    })
}

// ── Gathering ────────────────────────────────────────────────────────────

fn skip_dir(name: &str) -> bool {
    name == "target" || name == "node_modules" || (name.starts_with('.') && name != ".github")
}

fn walk(root: &Path, rel: &str, facts: &mut Facts) -> Result<(), String> {
    let dir = if rel.is_empty() {
        root.to_path_buf()
    } else {
        root.join(rel)
    };
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        let is_dir = entry.file_type().map_err(|e| format!("{path}: {e}"))?.is_dir();
        if is_dir {
            if skip_dir(&name) {
                continue;
            }
            walk(root, &path, facts)?;
            if rel.is_empty() {
                facts.top_dirs.insert(name);
            }
        } else if name == "Cargo.toml" {
            let text =
                std::fs::read_to_string(entry.path()).map_err(|e| format!("{path}: {e}"))?;
            let manifest: toml::Value =
                toml::from_str(&text).map_err(|e| format!("{path}: {e}"))?;
            if let Some(package) = manifest
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
            {
                facts.packages.insert(package.to_owned());
            }
        }
        facts.paths.insert(path);
    }
    Ok(())
}

/// Recipe names: unindented `name args…:` lines that are not `:=` assignments.
fn recipes(justfile: &str) -> BTreeSet<String> {
    justfile
        .lines()
        .filter(|line| !line.starts_with([' ', '\t', '#']) && !line.contains(":="))
        .filter_map(|line| {
            let head = line.split(':').next()?;
            let name = head.split_whitespace().next()?.trim_start_matches('@');
            (line.contains(':') && is_name(name)).then(|| name.to_owned())
        })
        .collect()
}

fn markdown_files(facts: &Facts) -> Vec<String> {
    facts
        .paths
        .iter()
        .filter(|p| {
            (p.starts_with("docs/content/") && p.ends_with(".md")) || p.as_str() == "README.md"
        })
        .cloned()
        .collect()
}

/// `cargo xtask docs`: gather, check every page, print the findings.
pub fn run_cli(root: &Path, xtask: BTreeSet<String>) -> Result<(), String> {
    let mut facts = Facts {
        xtask,
        ..Facts::default()
    };
    walk(root, "", &mut facts)?;
    let justfile = std::fs::read_to_string(root.join("justfile")).unwrap_or_default();
    facts.recipes = recipes(&justfile);

    let files = markdown_files(&facts);
    let mut findings = Vec::new();
    for file in &files {
        let text =
            std::fs::read_to_string(root.join(file)).map_err(|e| format!("{file}: {e}"))?;
        findings.extend(check_text(file, &text, &facts));
    }
    for finding in &findings {
        println!("{finding}");
    }
    println!(
        "docs: {} pages, {} names that do not exist",
        files.len(),
        findings.len()
    );
    if findings.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} doc references do not resolve (write other repos' paths as `task:apps/…`, \
             wrap sketches in <!-- doc-check: off --> … <!-- doc-check: on -->)",
            findings.len()
        ))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        let set = |items: &[&str]| items.iter().map(|s| (*s).to_owned()).collect();
        Facts {
            paths: set(&[
                "crates",
                "crates/architect",
                "crates/architect/src/lib.rs",
                "examples",
                "examples/greeter",
                "docs",
                "docs/content",
                "docs/content/architecture/peers.md",
            ]),
            top_dirs: set(&["crates", "examples", "docs"]),
            recipes: set(&["check", "tags"]),
            xtask: set(&["docs", "boundaries", "feature"]),
            packages: set(&["architect", "example-greeter"]),
        }
    }

    fn names(text: &str) -> Vec<(Kind, String)> {
        check_text("t.md", text, &facts())
            .into_iter()
            .map(|f| (f.kind, f.name))
            .collect()
    }

    #[test]
    fn existing_names_pass() {
        let text = "See `crates/architect/src/lib.rs:12-40` and `examples/greeter/`.\n\
                    ```sh\njust check\ncargo xtask boundaries\ncargo test -p example-greeter\n\
                    cargo run -q -p xtask -- docs\n```\n\
                    [peers](@/architecture/peers.md#who) and \
                    [src](https://github.com/FastTrackStudios/architect/blob/main/crates/architect)\n";
        assert_eq!(names(text), vec![]);
    }

    #[test]
    fn dead_names_fail_one_per_kind() {
        let text = "The demo is `examples/app/ui`.\n\
                    ```sh\n$ just test-e2e\ncargo xtask idioms\ncargo run -p app-server\n\
                    cargo run -p xtask -- ci\n```\n\
                    [gone](@/architecture/nope.md), \
                    [src](https://git.starcommand.live/codywright/architect/src/branch/main/examples/app/x.rs)\n";
        assert_eq!(
            names(text),
            vec![
                (Kind::Path, "examples/app/ui".to_owned()),
                (Kind::Recipe, "test-e2e".to_owned()),
                (Kind::Xtask, "idioms".to_owned()),
                (Kind::Package, "app-server".to_owned()),
                (Kind::Xtask, "ci".to_owned()),
                (Kind::Page, "docs/content/architecture/nope.md".to_owned()),
                (Kind::Path, "examples/app/x.rs".to_owned()),
            ]
        );
    }

    #[test]
    fn placeholders_other_repos_and_prose_are_not_paths() {
        let text = "`features/<f>/src` `crates/*` `task:apps/web` `studio:crates/daw` \
                    and crates/nope in prose, `/vox`, `https://example.com/a/b`.\n\
                    ```rust\nlet x = a / b; // just a sketch\n```\n";
        assert_eq!(names(text), vec![]);
    }

    #[test]
    fn off_regions_are_skipped() {
        let text = "<!-- doc-check: off (sketch) -->\n```\ncrates/timeline/\n```\n\
                    `cargo test -p inventory`\n<!-- doc-check: on -->\n`crates/gone`\n";
        assert_eq!(names(text), vec![(Kind::Path, "crates/gone".to_owned())]);
    }

    #[test]
    fn findings_carry_the_line() {
        let found = check_text("a.md", "one\n\n`crates/gone`\n", &facts());
        assert_eq!(found.first().map(|f| f.line), Some(3));
        assert_eq!(
            found.first().map(ToString::to_string).unwrap(),
            "a.md:3: path `crates/gone` does not exist"
        );
    }

    #[test]
    fn justfile_recipes_are_parsed() {
        let just = "set shell := [\"bash\"]\nx := \"1\"\n# a: comment\ndefault:\n    @just --list\n\
                    fleet-bump TAG +MANIFESTS:\n    echo\n@quiet:\n";
        let got = recipes(just);
        assert_eq!(
            got.into_iter().collect::<Vec<_>>(),
            vec!["default", "fleet-bump", "quiet"]
        );
    }

    #[test]
    fn a_span_that_wraps_keeps_its_parity() {
        let text = "There is no `cargo\nxtask` here, and `crates/gone` is code.\n\nprose crates/nope\n";
        assert_eq!(names(text), vec![(Kind::Path, "crates/gone".to_owned())]);
    }

    #[test]
    fn line_suffixes_strip() {
        assert_eq!(strip_line_suffix("a/b.rs:12-40,55"), "a/b.rs");
        assert_eq!(strip_line_suffix("a/b.rs:3:9"), "a/b.rs");
        assert_eq!(strip_line_suffix("a/b.rs"), "a/b.rs");
    }
}
