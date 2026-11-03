//! `EXT_CHECK` on the lab's clicker (#268 H5): its folder as the lab runs
//! it in development (the manifest beside a link to the program) checks
//! clean, its program started and its view rendered once; a deliberately
//! broken copy fails with the exact manifest field and line.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use architect_extensions::check::Checker;

/// The clicker as the lab's development copy is: its manifest and its
/// program, built for this test, under the name the manifest gives.
fn clicker(dir: &Path) -> PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        include_str!("../extension.toml"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-lab-clicker"),
        dir.join("architect-lab-clicker"),
    )
    .unwrap();
    dir.to_path_buf()
}

#[test]
fn the_clicker_checks_clean_and_a_broken_copy_names_the_field() {
    let tmp = std::env::temp_dir().join(format!("clicker-check-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let good = clicker(&tmp.join("good"));
    let report = Checker::new().check(&good);
    assert!(report.ok, "{report:?}");
    assert_eq!(report.id.as_deref(), Some("lab.clicker"));
    assert_eq!(report.kind.as_deref(), Some("process"));
    assert_eq!(report.views.len(), 1);
    let view = &report.views[0];
    assert!(view.rendered, "{view:?}");
    assert!(view.elements.is_some_and(|n| n >= 4), "{view:?}");

    // `effect = "read_only"` misspelt.
    let bad = clicker(&tmp.join("bad"));
    let manifest = bad.join("extension.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("effect = \"read_only\"", "effect = \"readonly\""),
    )
    .unwrap();
    let report = Checker::new().check(&bad);
    assert!(!report.ok);
    let p = &report.problems[0];
    assert_eq!(p.stage, "manifest", "{p:?}");
    assert_eq!(p.field.as_deref(), Some("effect"), "{p:?}");
    assert_eq!(p.line, Some(20), "{p:?}");
    assert!(p.message.contains("readonly"), "{p:?}");

    // A program that is not there: the load says so, at `entry`.
    let gone = clicker(&tmp.join("gone"));
    std::fs::remove_file(gone.join("architect-lab-clicker")).unwrap();
    let report = Checker::new().check(&gone);
    assert!(!report.ok);
    assert_eq!(report.problems[0].stage, "load", "{report:?}");
    let _ = std::fs::remove_dir_all(&tmp);
}
