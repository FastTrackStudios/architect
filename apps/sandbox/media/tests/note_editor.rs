//! The lab's note editor worked by a Luau extension (#312): `lab.outline`
//! (`apps/sandbox/extensions/outline`, loaded with `ARCHITECT_LAB_OUTLINE=1`)
//! in a Vault note's Edit mode. Tab and Shift-Tab move between a table's
//! cells and indent and outdent a list item, by the extension's keymaps;
//! one of its indents undoes as one step (Ctrl-Z); its decorations — a
//! mark on each TODO, a gutter marker on each list item and a widget that
//! mounts its count embed — are drawn, the editor's HTML matching
//! `tests/golden/note-editor-<state>.html` (`ARCHITECT_GOLDEN=record`
//! writes it again).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use architect_guide::run::{Desktop, Film, Pointer};

/// The note editor, as HTML, at the last frame shown.
#[derive(Default)]
struct EditorHtml(String);

impl Film for EditorHtml {
    fn frame(&mut self, desktop: &Desktop, _: Pointer) {
        let Some(main) = desktop.windows().first() else {
            return;
        };
        let doc = main.doc.inner.borrow();
        let Ok(found) = doc.query_selector_all("[data-note-editor]") else {
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

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("note-editor-{name}.html"))
}

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
            "note-editor-{name} does not look the same (diff {} {})",
            path.display(),
            got.display()
        );
    }
}

fn play(name: &str, steps: &str) -> String {
    let source = format!(
        "id note-editor-{name}\ntitle \"{name}\"\nview {{width 1280, height 800}}\n\
         capture {{fps 10, width 640}}\nsteps (\n{steps}    {{\n        id end\n        hold 0.3\n    }}\n)\n"
    );
    let scenario = architect_guide::load(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut film = EditorHtml::default();
    if let Err(e) =
        architect_guide::run::run(&architect_sandbox_media::app(), &scenario, Some(&mut film))
    {
        std::fs::write(format!("/tmp/ne312-{name}.html"), &film.0).ok();
        panic!("{name}: {e}");
    }
    film.0
}

/// The outline note open in the Vault, in its Edit mode, the extension's
/// keys loaded.
const OPEN: &str = r##"    {id vault, do @run{id LAB_GO_VAULT}, done {check ({id LAB_STATE, has {tab vault}})}}
    {id open, do @run{id WORKSPACE_OPEN, args {view note, id note-outline, title "Outline", at {replace {pane note-welcome}}}}}
    {id focus, do @run{id WORKSPACE_FOCUS, args {pane note-outline}}, done {check ({id WORKSPACE_LAYOUT, has {active note-outline}}), timeout 5}}
    {id edit, target @css"#note-edit", do @click{}}
    {id mounted, target @css"[data-note-editor=\"note-outline\"]", done {check ({id LAB_EDITOR_STATE, has {context note}}), timeout 5}}
"##;

/// Tab and Shift-Tab in the table: the next cell, the next row's, back.
const TABLE: &str = r"    {id in-day, do @run{id LAB_EDITOR_SELECT, args {anchor 84}}, done {check ({id LAB_EDITOR_STATE, has {node {kind table, cell {row 0, col 0}}}}), timeout 5}}
    {id tab-room, do @key{key Tab}, done {check ({id LAB_EDITOR_STATE, has {selection ({anchor 89, head 93})}}), timeout 5}}
    {id tab-mon, do @key{key Tab}, done {check ({id LAB_EDITOR_STATE, has {selection ({anchor 113, head 116})}}), timeout 5}}
    {id back-room, do @key{key Tab, mods (shift)}, done {check ({id LAB_EDITOR_STATE, has {selection ({anchor 89, head 93})}}), timeout 5}}
";

/// Tab and Shift-Tab in a list item; two indents, Ctrl-Z takes back one.
const LIST: &str = r"    {id in-setlist, do @run{id LAB_EDITOR_SELECT, args {anchor 56}}, done {check ({id LAB_EDITOR_STATE, has {node {kind list-item, indent 0}}}), timeout 5}}
    {id indent, do @key{key Tab}, done {check ({id LAB_EDITOR_STATE, has {node {kind list-item, indent 2}}}), timeout 5}}
    {id indent-again, do @key{key Tab}, done {check ({id LAB_EDITOR_STATE, has {node {kind list-item, indent 4}}}), timeout 5}}
    {id undo, do @key{key z, mods (ctrl)}, done {check ({id LAB_EDITOR_STATE, has {node {kind list-item, indent 2}}}), timeout 5}}
    {id outdent, do @key{key Tab, mods (shift)}, done {check ({id LAB_EDITOR_STATE, has {node {kind list-item, indent 0}}}), timeout 5}}
";

/// The note marked up: its decorations drawn, its widget mounted.
const MARK: &str = r#"    {id mark, do @run{id LAB_OUTLINE_MARK}}
    {id marked, target @css"[data-editor-mount] [data-embed=\"lab.outline/count\"]", done {timeout 5}}
"#;

#[test]
fn a_luau_extension_works_the_lab_s_note_editor() {
    let folder = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary sets them, before the lab (and
    // its threads) start.
    unsafe {
        std::env::set_var("ARCHITECT_SANDBOX_CONFIG_DIR", folder.path());
        std::env::set_var("ARCHITECT_LAB_OUTLINE", "1");
    }
    let html = play("keys", &format!("{OPEN}{TABLE}{LIST}{MARK}"));
    matches("marked", &html);
    assert!(
        html.contains("ext-mark ext-todo"),
        "the TODO's mark: {html}"
    );
    assert!(
        html.contains("editor-gutter ext-item"),
        "a gutter marker: {html}"
    );
    assert!(
        html.contains("data-editor-mount=\"ext-")
            && html.contains("data-embed=\"lab.outline/count\""),
        "the count widget, its embed mounted: {html}"
    );
}
