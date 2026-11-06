//! The default lab looks the same with its features as linked built-in
//! extensions (#272): each state below is the main window's document, as
//! HTML, at the end of a short scenario, and matches the one recorded in
//! `tests/golden/` from the lab before those features moved (the base of
//! the change: `git log -- apps/sandbox/media/tests/golden`).
//!
//! `ARCHITECT_GOLDEN=record` writes them again (only when the change is
//! meant to change how the lab looks).
//!
//! Each runs on a lab folder of its own, empty (`ARCHITECT_SANDBOX_CONFIG_DIR`):
//! nothing of the person's (their themes, layouts, extensions) is in it.
//! One test plays them in turn, as the folder is the process's. They are
//! the lab's default features: `architect-app/wasi` adds the manager's
//! Components kind to the extensions page.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use architect_guide::run::{Desktop, Film, Pointer};

/// The main window's document, as HTML, at the last frame shown.
#[derive(Default)]
struct Last(String);

impl Film for Last {
    fn frame(&mut self, desktop: &Desktop, _: Pointer) {
        if let Some(main) = desktop.windows().first() {
            self.0 = main.doc.inner.borrow().root_element().outer_html_pretty();
        }
    }
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("{name}.html"))
}

/// Plays `steps` over the lab and compares the window at the end with
/// `golden/<name>.html`.
fn looks_the_same(name: &str, steps: &str) {
    let source = format!(
        "id golden-{name}\ntitle \"{name}\"\nview {{width 1440, height 860}}\n\
         capture {{fps 10, width 960}}\nsteps (\n{steps}\n    {{\n        id end\n        hold 0.5\n    }}\n)\n"
    );
    let scenario = architect_guide::load(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
    let folder = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary sets it, between runs, before
    // the lab (and its threads) start.
    unsafe { std::env::set_var("ARCHITECT_SANDBOX_CONFIG_DIR", folder.path()) };
    let mut last = Last::default();
    architect_guide::run::run(&architect_sandbox_media::app(), &scenario, Some(&mut last))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    // The folder is a fresh one each run: its path is not how it looks.
    last.0 = last
        .0
        .replace(&folder.path().display().to_string(), "<lab folder>");
    let path = golden(name);
    if std::env::var("ARCHITECT_GOLDEN").as_deref() == Ok("record") {
        std::fs::write(&path, &last.0).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (ARCHITECT_GOLDEN=record)", path.display()));
    // An element's `data-dioxus-id` is the order it was mounted in, not
    // how it looks: a component that draws nothing moves every id after it.
    if without_ids(&want) != without_ids(&last.0) {
        let got = path.with_extension("got.html");
        std::fs::write(&got, &last.0).unwrap();
        let first = want
            .lines()
            .zip(last.0.lines())
            .position(|(a, b)| a != b)
            .unwrap_or_else(|| want.lines().count().min(last.0.lines().count()));
        panic!(
            "{name} does not look the same: first difference at line {} (diff {} {})",
            first.saturating_add(1),
            path.display(),
            got.display()
        );
    }
}

/// `html` with each `data-dioxus-id="…"` emptied.
fn without_ids(html: &str) -> String {
    const ID: &str = "data-dioxus-id=\"";
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(at) = rest.find(ID) {
        let (head, tail) = rest.split_at(at.saturating_add(ID.len()));
        out.push_str(head);
        rest = tail
            .find('"')
            .map_or("", |end| tail.get(end..).unwrap_or(""));
    }
    out.push_str(rest);
    out
}

/// The lab as it opens: the Inbox, its shell, its chrome.
fn the_lab_as_it_opens() {
    looks_the_same("inbox", "");
}

/// The settings page on its Appearance section: the themes to pick.
fn the_settings_page_appearance() {
    looks_the_same(
        "settings-appearance",
        r#"    {
        id open
        do @run{id APP_SETTINGS}
        hold 0.3
    }
    {
        id appearance
        target @text"Appearance"
        do @click{}
        hold 0.3
    }"#,
    );
}

/// The settings page on the extensions manager's section.
fn the_extensions_page() {
    looks_the_same(
        "settings-extensions",
        r#"    {
        id open
        do @run{id APP_SETTINGS}
        hold 0.3
    }
    {
        id appearance
        target @text"Appearance"
        do @click{}
        hold 0.3
    }
    {
        id extensions
        target @text"Extensions"
        do @click{}
        hold 0.3
    }"#,
    );
}

/// ⌘O asked only for extensions (`ext:`): the manager's items.
fn quick_open_extensions() {
    looks_the_same(
        "quick-open-ext",
        r#"    {
        id open
        do @run{id COMMAND_PALETTE_OPEN_ANYTHING}
        done {
            on (@changed"commands")
            check ({id COMMANDS_STATE, has {overlay quick-open}})
        }
        hold 0.3
    }
    {
        id find
        do @type{text "ext:"}
        hold 0.3
    }"#,
    );
}

#[test]
fn the_default_lab_looks_the_same() {
    the_lab_as_it_opens();
    the_settings_page_appearance();
    the_extensions_page();
    quick_open_extensions();
}
