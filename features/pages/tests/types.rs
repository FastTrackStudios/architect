//! The page API's generated types never drift: `architect.d.ts` and
//! `architect.d.luau` as checked in are what the descriptors make now.
//! `BLESS=1 cargo test -p architect-pages --features codegen --test types`
//! writes them again.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use architect_pages::codegen::{DTS, LUAU, luau, typescript, write_types};

fn checked_in(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("types")
        .join(name)
}

#[test]
fn the_checked_in_types_are_what_the_descriptors_make() {
    for (name, made) in [(DTS, typescript()), (LUAU, luau())] {
        let path = checked_in(name);
        if std::env::var_os("BLESS").is_some() {
            std::fs::write(&path, &made).unwrap();
        }
        let kept = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            kept == made,
            "{} drifted from the page API: run with BLESS=1 to write it again",
            path.display()
        );
    }
}

#[test]
fn the_types_name_every_method_and_the_runtime() {
    let ts = typescript();
    for name in [
        "export interface Element",
        "export interface PageEvent",
        "export interface PageHostCaller",
        "run(id: string, args: string)",
        "export interface PageViewCaller",
        "themeChanges(",
        "export function h(",
        "export function useData",
        "namespace JSX",
        "declare module \"architect/worker\"",
    ] {
        assert!(ts.contains(name), "architect.d.ts lacks {name}");
    }
    let lu = luau();
    assert!(lu.contains("export type Element = {"), "{lu}");
    assert!(lu.contains("\tkey: string?,"), "{lu}");
    assert!(lu.contains("\tchildren: { Element },"), "{lu}");
}

#[test]
fn an_app_gets_both_files_in_its_folder() {
    let app = tempfile::tempdir().unwrap();
    let written = write_types(app.path()).unwrap();
    assert_eq!(written.len(), 2);
    assert_eq!(
        std::fs::read_to_string(app.path().join(DTS)).unwrap(),
        typescript()
    );
}
