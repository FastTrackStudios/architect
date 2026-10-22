//! `cargo xtask feature new` end to end: scaffold a feature into a copy
//! of this workspace and `cargo check` what it wrote, with no features
//! and for `wasm32-unknown-unknown` (the contract must be wasm-clean),
//! then with its backend, vox face and test target.
//!
//! The copy keeps this repo's `[workspace.dependencies]`, patches,
//! lints, profiles and `Cargo.lock` (internal paths made absolute), and
//! builds into this repo's own target dir, so it reuses the warm cache
//! instead of creating a second one.

#![allow(clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const NAME: &str = "scaffold-probe";

#[test]
fn scaffolds_one_crate_that_checks_natively_and_for_wasm() {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace root")
        .to_path_buf();
    let workspace = fresh_workspace(&repo);

    let out = Command::new(env!("CARGO_BIN_EXE_xtask"))
        .args(["feature", "new", NAME])
        .current_dir(&workspace)
        .output()
        .expect("run xtask");
    assert!(
        out.status.success(),
        "xtask feature new failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // One crate, nothing beside it.
    let feature = workspace.join("features").join(NAME);
    for file in [
        "Cargo.toml",
        "src/lib.rs",
        "src/memory.rs",
        "src/ui.rs",
        "spec/service.md",
        "tests/memory.rs",
    ] {
        assert!(feature.join(file).is_file(), "missing {file}");
    }
    let siblings: Vec<_> = fs::read_dir(&feature)
        .expect("read feature dir")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(NAME))
        .collect();
    assert!(siblings.is_empty(), "sibling crates written: {siblings:?}");

    let manifest = fs::read_to_string(workspace.join("Cargo.toml")).expect("read manifest");
    assert!(manifest.contains(&format!("\"features/{NAME}\",")));
    assert!(manifest.contains(&format!(
        "{NAME} = {{ path = \"features/{NAME}\", default-features = false }}"
    )));

    if repo.join("boundaries.toml").exists() {
        let table = fs::read_to_string(workspace.join("boundaries.toml")).expect("read table");
        assert_eq!(
            table.matches(&format!("package = \"{NAME}\"")).count(),
            4,
            "one boundaries row per role"
        );
    }

    // The contract alone, natively and for the browser.
    cargo_check(&repo, &workspace, &[]);
    cargo_check(&repo, &workspace, &["--target", "wasm32-unknown-unknown"]);
    // The backend, the vox face and the scenario test (its dev-dependency
    // on the crate itself turns `memory` and `vox` on).
    cargo_check(&repo, &workspace, &["--all-targets"]);
}

/// A copy of this workspace's root manifest with no members, in a fresh
/// directory under cargo's per-test scratch dir.
fn fresh_workspace(repo: &Path) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xtask-feature-new");
    if dir.exists() {
        fs::remove_dir_all(&dir).expect("clear the previous run");
    }
    fs::create_dir_all(&dir).expect("create workspace dir");

    let root = fs::read_to_string(repo.join("Cargo.toml")).expect("read root manifest");
    let absolute = format!("path = \"{}/", repo.display());
    let manifest = without_members(&root).replace("path = \"", &absolute);
    fs::write(dir.join("Cargo.toml"), manifest).expect("write manifest");
    for file in ["Cargo.lock", "rust-toolchain.toml", "boundaries.toml"] {
        let from = repo.join(file);
        if from.exists() {
            fs::copy(&from, dir.join(file)).expect("copy workspace file");
        }
    }
    dir
}

/// `members = [ ... ]` emptied, everything else as it was.
fn without_members(manifest: &str) -> String {
    let mut out = Vec::new();
    let mut skipping = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if !skipping && trimmed.starts_with("members") && trimmed.ends_with('[') {
            out.push("members = [");
            skipping = true;
            continue;
        }
        if skipping {
            if trimmed.starts_with(']') {
                out.push("]");
                skipping = false;
            }
            continue;
        }
        out.push(line);
    }
    out.join("\n") + "\n"
}

fn cargo_check(repo: &Path, workspace: &Path, extra: &[&str]) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.args(["check", "-p", NAME])
        .args(extra)
        .current_dir(workspace);
    // The repo's own target dir (or the one the caller chose), never a
    // new one: the scaffold's dependencies are already built there.
    if std::env::var_os("CARGO_TARGET_DIR").is_none() {
        cmd.arg("--target-dir").arg(repo.join("target"));
    }
    let out = cmd.output().expect("run cargo check");
    assert!(
        out.status.success(),
        "cargo check -p {NAME} {extra:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
