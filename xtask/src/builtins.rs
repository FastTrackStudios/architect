//! `cargo xtask builtins` — one trait, three ways to run it (#272).
//!
//! Every built-in extension crate (an `extension.toml` beside a
//! `Cargo.toml` whose manifest says where its crate is, `[source]`) is one
//! `linked::Extension` value that the app links in and that its binary
//! serves as a process. So each must build both ways:
//!
//! 1. **linked**: `cargo build -p <crate> --lib` (what an app links in);
//! 2. **process**: `cargo build -p <crate> --bin <bin>` (what `EXT_CLONE`
//!    builds in the development folder, `[source] bin` or the
//!    `[entry] path`'s file name);
//! 3. **WASI**: not built here. The WASI runner (PR #285) runs components
//!    written against their own guest crate (`architect-extension-wasi`,
//!    the lab's `apps/lab/wasi-counter`), not a `linked::Extension`. The
//!    third way waits on a `linked::Extension` exported as a component
//!    (#272); the components themselves are built and run by
//!    `features/extensions/tests/wasi.rs`.
//!
//! The Board (`apps/lab/board`) is the first.

use std::path::{Path, PathBuf};
use std::process::Command;

/// One built-in extension crate.
#[derive(Debug, PartialEq, Eq)]
pub struct Builtin {
    pub dir: PathBuf,
    pub package: String,
    pub bin: String,
}

/// Every built-in extension crate under `root`, in path order.
pub fn find(root: &Path) -> Vec<Builtin> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

fn walk(dir: &Path, out: &mut Vec<Builtin>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        let path = entry.path();
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            walk(&path, out);
        } else if name == "extension.toml" {
            if let Some(b) = builtin(dir) {
                out.push(b);
            }
        }
    }
}

fn builtin(dir: &Path) -> Option<Builtin> {
    let read = |f: &str| {
        std::fs::read_to_string(dir.join(f))
            .ok()
            .and_then(|t| toml::from_str::<toml::Table>(&t).ok())
    };
    let manifest = read("extension.toml")?;
    let source = manifest.get("source")?.as_table()?;
    let cargo = read("Cargo.toml")?;
    let package = cargo.get("package")?.get("name")?.as_str()?.to_owned();
    let bin = source
        .get("bin")
        .and_then(toml::Value::as_str)
        .filter(|b| !b.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            let entry = manifest.get("entry")?.get("path")?.as_str()?;
            Some(Path::new(entry).file_name()?.to_string_lossy().into_owned())
        })?;
    Some(Builtin {
        dir: dir.to_path_buf(),
        package,
        bin,
    })
}

/// Builds each built-in crate under `root` linked and as a process.
pub fn run_cli(root: &Path) -> Result<(), String> {
    let builtins = find(root);
    if builtins.is_empty() {
        return Err("no built-in extension crate ([source] in an extension.toml)".to_owned());
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut failed = Vec::new();
    for b in &builtins {
        let ways: [(&str, Vec<&str>); 2] = [
            ("linked", vec!["build", "-q", "-p", &b.package, "--lib"]),
            (
                "process",
                vec!["build", "-q", "-p", &b.package, "--bin", &b.bin],
            ),
            // No WASI way yet: see the module's doc.
        ];
        for (way, args) in ways {
            let ok = Command::new(&cargo)
                .current_dir(root)
                .args(&args)
                .status()
                .is_ok_and(|s| s.success());
            println!(
                "{} {} ({way}): {}",
                if ok { "ok  " } else { "FAIL" },
                b.package,
                args.join(" ")
            );
            if !ok {
                failed.push(format!("{} {way}", b.package));
            }
        }
    }
    if failed.is_empty() {
        println!(
            "{} built-in extension crate(s) build linked and as a process",
            builtins.len()
        );
        Ok(())
    } else {
        Err(format!("did not build: {}", failed.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_board_is_a_built_in_crate() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let found = super::find(&root);
        assert!(
            found
                .iter()
                .any(|b| b.package == "architect-lab-board" && b.bin == "architect-lab-board"),
            "{found:?}"
        );
    }
}
