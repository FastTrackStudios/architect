//! Pages in the extensions manager (`pages` kind), in `APP_STATUS`'s
//! problems and in `EXT_CHECK` (`architect_pages::manage`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use architect::action::{ActionList, Problems};
use architect_extensions::check::Checker;
use architect_extensions::manager::{ItemKind as _, Manager};
use architect_pages::files::{Origin, PageFile, write_builtin};
use architect_pages::host::AppHost;
use architect_pages::manage::PagesKind;
use architect_pages::runner::Pages;
use serde_json::Value;

const GOOD: &str = r#"import { h } from "architect";
export default function Board() {
  return <div class="board"><span>pinned</span></div>;
}
"#;

const BROKEN: &str = "export default function Board() {\n  return <div>;\n}\n";

fn pages(dir: &Path) -> Pages {
    Pages::new(AppHost::new(ActionList::new()))
        .with_app_dir(dir)
        .with_js()
}

fn ids(kind: &PagesKind) -> Vec<String> {
    kind.list().into_iter().map(|i| i.id).collect()
}

fn ops(kind: &PagesKind, id: &str) -> Vec<String> {
    kind.list()
        .into_iter()
        .find(|i| i.id == id)
        .unwrap()
        .ops
        .into_iter()
        .map(|o| o.id)
        .collect()
}

/// A built-in's page is listed read-only with clone-to-edit; cloned, the
/// override wins and offers its removal; broken, it is a problem in the
/// manager and in `APP_STATUS`; removed, the built-in shows again.
#[test]
fn the_manager_lists_pages_clones_and_restores_them() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = pages(tmp.path());
    let dir = write_builtin(
        tmp.path(),
        "lab.board",
        &[
            ("board.tsx", GOOD),
            ("primes.js", "export const p = [2, 3];\n"),
        ],
    )
    .unwrap();
    pages.declare(
        "lab.board",
        "board",
        PageFile::new(dir.join("board.tsx"), Origin::Builtin).unwrap(),
    );
    let manager = Manager::new().with(PagesKind::new(pages.clone()));
    let kind = manager.kind("pages").unwrap();
    let problems = Problems::default();
    pages.report_problems(&problems);
    let pages_kind = PagesKind::new(pages);

    let listed = manager.list(Some("pages"), "");
    assert_eq!(listed.len(), 1, "{listed:?}");
    let board = &listed[0];
    assert_eq!(board.id, "lab.board/board");
    assert_eq!(board.source, "built in");
    assert_eq!(ops(&pages_kind, "lab.board/board"), ["check", "clone"]);
    let checked = kind.run("lab.board/board", "check", &Value::Null).unwrap();
    assert!(checked["nodes"].as_u64().unwrap() >= 2, "{checked}");

    // Clone to edit: the override wins; what it imports came along, and is
    // not listed as a view of its own.
    let cloned = kind.run("lab.board/board", "clone", &Value::Null).unwrap();
    let path = PathBuf::from(cloned["path"].as_str().unwrap());
    assert_eq!(path, tmp.path().join("pages/lab.board/board.tsx"));
    assert!(tmp.path().join("pages/lab.board/primes.js").is_file());
    assert_eq!(ids(&pages_kind), ["lab.board/board"]);
    let item = &manager.list(Some("pages"), "")[0];
    assert_eq!(item.source, "an override");
    assert_eq!(ops(&pages_kind, "lab.board/board"), ["check", "restore"]);
    assert!(manager.is_destructive("pages", "lab.board/board", "restore"));

    // Broken: its `.error` (as its thread writes it) is its problem.
    std::fs::write(&path, BROKEN).unwrap();
    let why = kind
        .run("lab.board/board", "check", &Value::Null)
        .unwrap_err();
    std::fs::write(
        architect_pages::files::error_path(&path),
        format!("{why}\n"),
    )
    .unwrap();
    let item = &manager.list(Some("pages"), "")[0];
    assert_eq!(item.problems.len(), 1, "{item:?}");
    let all = problems.all();
    assert_eq!(all.len(), 1, "{all:?}");
    assert_eq!(all[0].source, "page");
    assert_eq!(all[0].field.as_deref(), Some("lab.board/board"));
    assert_eq!(all[0].file.as_deref(), Some(path.to_str().unwrap()));
    let details = manager.get("pages", "lab.board/board").unwrap();
    assert!(
        details
            .parts
            .iter()
            .any(|p| p.title == "Why it is not the page as saved")
    );

    // Restored: the built-in again, no problem.
    kind.run("lab.board/board", "restore", &Value::Null)
        .unwrap();
    assert!(!path.exists());
    let item = &manager.list(Some("pages"), "")[0];
    assert_eq!(item.source, "built in");
    assert!(problems.all().is_empty());
}

/// A running page that is not the page as saved is in `APP_STATUS`.
#[test]
fn a_broken_running_page_is_an_app_problem() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = pages(tmp.path());
    let problems = Problems::default();
    pages.report_problems(&problems);
    let path = tmp.path().join("pages/lab.test/page.tsx");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, BROKEN).unwrap();
    let page = PageFile::new(&path, Origin::Override).unwrap();
    let session = pages.open(&page, "lab.test", "page", None).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while problems.all().is_empty() && std::time::Instant::now() < until {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let all = problems.all();
    assert_eq!(all.len(), 1, "{all:?}");
    assert!(
        all[0].message.starts_with("lab.test/page: "),
        "{}",
        all[0].message
    );
    drop(session);
}

fn extension_with_page(dir: &Path, page: &str) -> PathBuf {
    let folder = dir.join("ext");
    std::fs::create_dir_all(folder.join("pages")).unwrap();
    std::fs::write(
        folder.join("extension.toml"),
        r#"id = "lab.paged"
name = "Paged"
version = "0.1.0"
kind = "luau"
api = "0.1"
description = "A view drawn from a page file."

[[views]]
id = "board"
title = "Board"
page = "pages/board.tsx"
"#,
    )
    .unwrap();
    std::fs::write(folder.join("pages/board.tsx"), page).unwrap();
    folder
}

/// `EXT_CHECK` renders an extension's page file once through the app's
/// pages, and a broken one is its problem, with the file and field.
#[test]
fn ext_check_checks_an_extensions_page_files() {
    let tmp = tempfile::tempdir().unwrap();
    let pages = pages(tmp.path());
    let checker = Checker::new().with_page_checker(pages.page_checker());

    let good = checker.check(&extension_with_page(tmp.path(), GOOD));
    assert!(good.ok, "{}", good.summary());
    let view = good.views.iter().find(|v| v.id == "board").unwrap();
    assert!(view.rendered, "{view:?}");
    assert!(view.elements.unwrap() >= 2, "{view:?}");

    let bad = checker.check(&extension_with_page(tmp.path(), BROKEN));
    assert!(!bad.ok);
    let problem = &bad.problems[0];
    assert_eq!(problem.stage, "page");
    assert_eq!(problem.file.as_deref(), Some("pages/board.tsx"));
    assert_eq!(problem.field.as_deref(), Some("views[board].page"));

    // Without the app's pages, the page is said not to be checked.
    let unchecked = Checker::new().check(&extension_with_page(tmp.path(), BROKEN));
    assert!(unchecked.ok, "{}", unchecked.summary());
    let note = unchecked.views[0].note.clone().unwrap();
    assert!(note.contains("not checked"), "{note}");
}
