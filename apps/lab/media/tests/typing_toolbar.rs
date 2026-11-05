//! The Typing Toolbar on a phone (#316): the bar over the keyboard while
//! a field is typed in. Each state's bar, as HTML, matches
//! `tests/golden/typing-<state>.html`; `ARCHITECT_GOLDEN=record` writes
//! them again.
//!
//! Each run is on a lab folder of its own (`ARCHITECT_LAB_CONFIG_DIR`);
//! one test plays them in turn, as the folder is the process's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use architect_guide::run::{Desktop, Film, Pointer};

/// The phone's bar at the foot, as HTML, at the last frame shown.
#[derive(Default)]
struct Bar(String);

impl Film for Bar {
    fn frame(&mut self, desktop: &Desktop, _: Pointer) {
        let Some(main) = desktop.windows().first() else {
            return;
        };
        let doc = main.doc.inner.borrow();
        let Ok(found) = doc.query_selector_all("[data-bar]") else {
            return;
        };
        self.0 = found
            .iter()
            .filter_map(|id| doc.get_node(*id))
            .map(|n| without_ids(&n.outer_html_pretty()))
            .collect::<Vec<_>>()
            .join("\n");
    }
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("typing-{name}.html"))
}

/// `html` with each `data-dioxus-id="…"` emptied: the order things were
/// mounted in, not how they look.
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

/// Plays `steps` on a phone's screen in `folder` and gives the bar at the
/// end.
fn play(name: &str, folder: &Path, steps: &str) -> String {
    let source = format!(
        "id typing-{name}\ntitle \"{name}\"\nview {{width 402, height 874}}\n\
         capture {{fps 10, width 402}}\nsteps (\n{steps}    {{\n        id end\n        hold 0.4\n    }}\n)\n"
    );
    let scenario = architect_guide::load(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
    // SAFETY: the only test in this binary sets it, between runs, before
    // the lab (and its threads) start.
    unsafe { std::env::set_var("ARCHITECT_LAB_CONFIG_DIR", folder) };
    let mut film = Bar::default();
    if let Err(e) =
        architect_guide::run::run(&architect_lab_media::app(), &scenario, Some(&mut film))
    {
        std::fs::write(format!("/tmp/tt316-{name}.html"), &film.0).ok();
        panic!("{name}: {e}");
    }
    film.0
}

/// `html` is `golden/typing-<name>.html`.
fn matches(name: &str, html: &str) {
    let path = golden(name);
    if std::env::var("ARCHITECT_GOLDEN").as_deref() == Ok("record") {
        std::fs::write(&path, html).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (ARCHITECT_GOLDEN=record)", path.display()));
    if want != html {
        let got = path.with_extension("got.html");
        std::fs::write(&got, html).unwrap();
        panic!(
            "typing-{name} does not look the same (diff {} {})",
            path.display(),
            got.display()
        );
    }
}

/// The capture field typed in: the bar over the keyboard.
const CAPTURE: &str = "    {id capture, do @run{id LAB_CAPTURE}, done {check ({id LAB_STATE, has {page capture, bar editing}})}}
";

/// The Board's chart open in the Vault, its chord line typed in: the
/// Board's Typing Toolbar.
const SHEET: &str = r##"    {id vault, do @run{id LAB_GO_VAULT}, done {on (@changed"lab"), check ({id LAB_STATE, has {tab vault}})}}
    {id sheet, do @run{id EXT_VIEW_OPEN, args {extension lab.board, view chart}}, done {check ({id WORKSPACE_LAYOUT, has {active "lab.board/chart"}}), timeout 5}}
    {id field, target @css"#chords-text", do @click{}, done {check ({id LAB_STATE, has {bar editing}})}}
    {id shown, target @css"[data-bar-item=\"LAB_BOARD_CHORD_C\"]", done {timeout 5}}
"##;

/// The sharps toggle pressed, then C: the toggle shows on, the chord is
/// a sharp, and the field keeps the focus (the bar is still the typing
/// one).
const SHARP_C: &str = r#"    {id sharp, target @css"[data-bar-item=\"LAB_BOARD_SHARP\"]", do @click{}, done {check ({id LAB_BOARD_CHORDS, has {sharp true}} {id LAB_STATE, has {bar editing}})}}
    {id on, target @css"[data-bar-item=\"LAB_BOARD_SHARP\"][aria-pressed=\"true\"]", done {timeout 5}}
    {id c, target @css"[data-bar-item=\"LAB_BOARD_CHORD_C\"]", do @click{}, done {check ({id LAB_BOARD_CHORDS, has {text "C#"}} {id LAB_STATE, has {bar editing}})}}
"#;

/// The sheet cleared; the bar scrolled sideways to its end, its dropdown
/// opened above it, a row pressed (the field keeps the focus), and opened
/// again.
const SCROLL_AND_INSERT: &str = r#"    {id clean, do @run{id LAB_BOARD_CHORDS_CLEAR}, done {check ({id LAB_BOARD_CHORDS, has {text ""}})}}
    {id scroll, target @css"[data-bar-item=\"LAB_BOARD_CHORD_E\"]", do @drag{dx -320, seconds 0.6}}
    {id scrolled, target @css"[data-bar-item=\"widget:lab.board/tempo\"]", done {check ({id LAB_STATE, has {bar editing}})}}
    {id insert, target @css"[data-bar-item=\"menu:lab.board/chords#8\"]", do @click{}}
    {id opened, target @css"[data-typing-menu] [data-tool=\"LAB_BOARD_BAR_LINE\"]", done {timeout 5}}
    {id bar-line, target @css"[data-typing-menu] [data-tool=\"LAB_BOARD_BAR_LINE\"]", do @click{}, done {check ({id LAB_BOARD_CHORDS, has {text "|"}} {id LAB_STATE, has {bar editing}})}}
    {id again, target @css"[data-bar-item=\"menu:lab.board/chords#8\"]", do @click{}}
    {id open-again, target @css"[data-typing-menu]", done {timeout 5}}
"#;

/// A person's edit to the capture field's own items, with the editor's
/// actions: Clear moved first, Tag taken away.
const EDIT: &str =
    "    {id clear-first, do @run{id TOOLBAR_MOVE, args {id typing/capture, from 8, to 0}}}
    {id no-tag, do @run{id TOOLBAR_REMOVE, args {id typing/capture, at 5}}}
";

#[test]
fn the_typing_toolbar_on_a_phone() {
    // The editor's own items, as the bar showed them before it was a
    // toolbar: unchanged.
    let folder = tempfile::tempdir().unwrap();
    let builtins = play("builtins", folder.path(), CAPTURE);
    matches("builtins", &builtins);
    assert!(
        builtins.contains(r#"data-bar-item="LAB_CAPTURE_TAG""#),
        "{builtins}"
    );
    assert!(
        builtins.contains(r#"data-bar-item="hide-keyboard""#),
        "{builtins}"
    );
    assert!(
        !builtins.contains("LAB_BOARD_CHORD"),
        "the Board's items show only while its chart has the focus"
    );

    // The Board's chart has the focus: its items, not the capture's; a
    // toggle showing on; more than fit, the rest past the pill's end.
    let chords = play("chords", folder.path(), &format!("{SHEET}{SHARP_C}"));
    matches("chords", &chords);
    assert!(!chords.contains("LAB_CAPTURE_"), "{chords}");
    assert!(
        chords.contains(r#"aria-pressed="true""#)
            && chords.contains(r#"data-bar-item="LAB_BOARD_CHORD_C""#),
        "{chords}"
    );
    assert!(
        !chords.contains("widget:lab.board/tempo"),
        "past the end: {chords}"
    );

    // Scrolled sideways: the dropdown and the widget come into view; the
    // dropdown opens above the bar.
    let menu = play(
        "chords-menu",
        folder.path(),
        &format!("{SHEET}{SCROLL_AND_INSERT}"),
    );
    matches("chords-menu", &menu);
    assert!(
        menu.contains(r#"data-bar-item="widget:lab.board/tempo""#),
        "{menu}"
    );
    assert!(
        menu.contains(r#"data-typing-menu="lab.board/chords#8""#),
        "{menu}"
    );
    assert!(
        !menu.contains(r#"data-bar-item="LAB_BOARD_SHARP""#),
        "scrolled out: {menu}"
    );

    // A person's edit, written and read again after a restart: Clear
    // first, no Tag.
    let mine = tempfile::tempdir().unwrap();
    let _ = play("edit", mine.path(), &format!("{CAPTURE}{EDIT}"));
    assert!(
        has_file(mine.path(), "typing-capture.toml"),
        "the person's file"
    );
    let edited = play("edited", mine.path(), CAPTURE);
    matches("edited", &edited);
    assert!(!edited.contains("LAB_CAPTURE_TAG"), "{edited}");
    let (clear, add) = (
        edited.find(r#"data-bar-item="LAB_CAPTURE_CLEAR""#),
        edited.find(r#"data-bar-item="LAB_CAPTURE_ADD""#),
    );
    assert!(clear.is_some() && clear < add, "{edited}");

    // The toolbar editor on: the bar's toolbars, editable, above it.
    let editor = play(
        "editor",
        mine.path(),
        &format!(
            "{CAPTURE}    {{id edit, do @run{{id TOOLBAR_EDIT, args {{on true}}}}}}\n    {{id strip, target @css\"[data-typing-above] [data-toolbar=\\\"typing/capture\\\"]\", done {{timeout 5}}}}\n"
        ),
    );
    matches("editor", &editor);
}

/// Whether a file called `name` is anywhere under `dir`.
fn has_file(dir: &Path, name: &str) -> bool {
    std::fs::read_dir(dir).is_ok_and(|entries| {
        entries.flatten().any(|e| {
            let p = e.path();
            p.file_name().is_some_and(|n| n == name) || (p.is_dir() && has_file(&p, name))
        })
    })
}
