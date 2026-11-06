//! The editor API (#312): the lab's `lab.outline` extension
//! (`apps/sandbox/extensions/outline`, Luau) works an editor through the host
//! service — its keymaps pick its actions by the block at the cursor
//! (Tab/Shift-Tab indent and outdent a list item, move between a table's
//! cells), each edit is one undo step made as the extension, its
//! decorations land (a widget naming its own embed), its slash row is
//! there; an extension without `writes = ["editor"]` is refused.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use architect::action::{ActionCaller, ActionList};
use architect_extensions::editor::{
    DecorationSpec, KeyChord, MemoryEditor, SelRange, key_action, node_at,
};
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::{Origin, Package};
use architect_settings::Registry;
use futures::executor::block_on;
use serde_json::{Value, json};

const NOTE: &str = "- one\n- two\n\n| a | b |\n|---|---|\n| c | d |";

fn outline_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/sandbox/extensions/outline")
}

fn host(editor: &Arc<MemoryEditor>) -> (ActionList, Host) {
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.set_editors(editor.clone());
    let package = Package::read(&outline_dir()).unwrap();
    host.load(&package, Origin::Dev).unwrap();
    (list, host)
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    block_on(list.run_json_async_with(&ActionCaller::app(), id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

/// What the app does with a key in the editor: the extension's keymap
/// entry for it at the cursor, its action run.
fn press(list: &ActionList, host: &Host, editor: &MemoryEditor, key: &str) -> String {
    let view = editor.view();
    let node = node_at(&view.snapshot.text, view.snapshot.cursor);
    let mut chord = KeyChord::parse(key);
    // A keystroke, not a shortcut: `Shift-Tab` is Tab with Shift held.
    chord.r#mod = false;
    let hit = key_action(host.contributions(), &chord, &node)
        .unwrap_or_else(|| panic!("no key {key} at {:?}", node.kind));
    assert_eq!(hit.owner, "lab.outline");
    run(list, &hit.item.action, &Value::Null).unwrap();
    hit.item.action
}

fn selection(editor: &MemoryEditor) -> (usize, usize) {
    let s = editor.view().snapshot.selection[0];
    (s.anchor, s.head)
}

#[test]
fn tab_and_shift_tab_indent_a_list_item_and_move_between_cells() {
    let editor = Arc::new(MemoryEditor::new("lab.note/Inbox.md", "note", NOTE));
    let (list, host) = host(&editor);

    // In the list: Tab indents "two", one undo step, as the extension.
    editor.put_caret(NOTE.find("two").unwrap());
    assert_eq!(press(&list, &host, &editor, "Tab"), "LAB_OUTLINE_INDENT");
    assert!(
        editor.text().starts_with("- one\n  - two\n"),
        "{}",
        editor.text()
    );
    assert_eq!(
        selection(&editor).1,
        NOTE.find("two").unwrap() + 2,
        "the caret kept its text"
    );
    let steps = editor.steps();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].name, "Indent list item");
    assert!(steps[0].by.contains("lab.outline"), "{}", steps[0].by);
    // Shift-Tab takes it back out.
    assert_eq!(
        press(&list, &host, &editor, "Shift-Tab"),
        "LAB_OUTLINE_OUTDENT"
    );
    assert_eq!(editor.text(), NOTE);

    // In the table: Tab selects the next cell, Shift-Tab the previous.
    let a = NOTE.find("| a").unwrap() + 2;
    editor.put_caret(a);
    assert_eq!(press(&list, &host, &editor, "Tab"), "LAB_OUTLINE_NEXT_CELL");
    let b = NOTE.find('b').unwrap();
    assert_eq!(selection(&editor), (b, b + 1));
    // b's row is the header; past it, c on the next row (the delimiter
    // row skipped).
    press(&list, &host, &editor, "Tab");
    let c = NOTE.find('c').unwrap();
    assert_eq!(selection(&editor), (c, c + 1));
    press(&list, &host, &editor, "Shift-Tab");
    assert_eq!(selection(&editor), (b, b + 1));
    // Past the last cell: a new row, the caret in its first cell.
    editor.put_caret(NOTE.find('d').unwrap());
    press(&list, &host, &editor, "Tab");
    assert_eq!(editor.text(), format!("{NOTE}\n|   |   |"));
    assert_eq!(selection(&editor), (NOTE.len() + 3, NOTE.len() + 3));
    assert_eq!(
        node_at(&editor.text(), NOTE.len() + 3)
            .cell
            .map(|c| (c.row, c.col)),
        Some((2, 0))
    );
}

#[test]
fn an_extensions_edit_undoes_as_one_step_and_leaves_the_persons_typing() {
    let editor = Arc::new(MemoryEditor::new("lab.note/Inbox.md", "note", "- a\n- b"));
    let (list, host) = host(&editor);
    editor.type_text(" and more");
    // The table, a three-line insert and a selection, in one step.
    editor.put_caret(0);
    run(&list, "LAB_OUTLINE_TABLE", &Value::Null).unwrap();
    assert_eq!(
        editor.text(),
        "| A | B |\n|---|---|\n|   |   |\n- a\n- b and more"
    );
    assert_eq!(selection(&editor), (2, 3), "its first cell's text selected");
    assert_eq!(editor.undo().as_deref(), Some("Insert a table"));
    assert_eq!(
        editor.text(),
        "- a\n- b and more",
        "the person's typing kept"
    );
    assert!(editor.steps().is_empty());
    drop(host);
}

#[test]
fn its_decorations_land_and_a_widget_names_its_own_embed() {
    let editor = Arc::new(MemoryEditor::new(
        "lab.note/Inbox.md",
        "note",
        "TODO call\n- a\n- b TODO",
    ));
    let (list, host) = host(&editor);
    assert_eq!(
        run(&list, "LAB_OUTLINE_MARK", &Value::Null).unwrap(),
        json!(2)
    );
    let all = editor.decorations();
    let mine = &all["lab.outline"];
    let marks = mine
        .iter()
        .filter(|d| matches!(d, DecorationSpec::Mark { class, .. } if class == "todo"))
        .count();
    let gutters = mine
        .iter()
        .filter(|d| matches!(d, DecorationSpec::Gutter { .. }))
        .count();
    assert_eq!((marks, gutters), (2, 2), "{mine:?}");
    assert!(
        mine.iter().any(|d| matches!(d,
            DecorationSpec::Widget { view, props, block: true, .. }
                if view == "count" && props["items"] == 2)),
        "{mine:?}"
    );
    // They follow the text.
    editor.put_caret(0);
    editor.type_text(">> ");
    let moved = &editor.decorations()["lab.outline"];
    assert_eq!(moved.first().map(DecorationSpec::at), Some(3));
    // Cleared, and gone with the extension when it unloads.
    run(&list, "LAB_OUTLINE_MARK", &json!({ "clear": true })).unwrap();
    assert!(editor.decorations().is_empty());
    run(&list, "LAB_OUTLINE_MARK", &Value::Null).unwrap();
    assert!(host.unload("lab.outline"));
    assert!(editor.decorations().is_empty());
}

#[test]
fn its_slash_row_and_keymaps_are_contributions() {
    let editor = Arc::new(MemoryEditor::new("e", "note", ""));
    let (_list, host) = host(&editor);
    let slash = host.contributions().slash();
    assert_eq!(slash.len(), 1);
    assert_eq!(
        (slash[0].item.label.as_str(), slash[0].item.action.as_str()),
        ("Table", "LAB_OUTLINE_TABLE")
    );
    assert_eq!(host.contributions().keymaps().len(), 4);
    assert!(host.unload("lab.outline"));
    assert!(host.contributions().keymaps().is_empty());
}

/// A Luau extension of `capabilities` (TOML) whose action runs `script`.
fn package(dir: &Path, capabilities: &str, extra: &str, script: &str) -> Package {
    try_package(dir, capabilities, extra, script).unwrap()
}

fn try_package(
    dir: &Path,
    capabilities: &str,
    extra: &str,
    script: &str,
) -> Result<Package, String> {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        format!(
            "id = \"lab.nosy\"\nname = \"Nosy\"\nversion = \"0.1.0\"\nkind = \"luau\"\napi = \"0.1\"\n\
             [capabilities]\n{capabilities}\n\
             [[actions]]\nid = \"LAB_NOSY_GO\"\nname = \"Go\"\nscript = \"go.luau\"\n\
             params = '{{\"type\":\"object\",\"properties\":{{\"which\":{{\"type\":\"integer\"}}}}}}'\n{extra}"
        ),
    )
    .unwrap();
    std::fs::write(dir.join("go.luau"), script).unwrap();
    Package::read(dir).map_err(|e| e.to_string())
}

#[test]
fn without_the_capability_it_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let editor = Arc::new(MemoryEditor::new("e", "note", "- a"));
    let list = ActionList::new();
    let host = Host::new(list.clone(), SharedSections::new(Registry::new()));
    host.set_editors(editor.clone());

    // Reading it needs reads (or writes) = ["editor"].
    let p = package(&tmp.path().join("a"), "", "", "return host.editor()");
    host.load(&p, Origin::Dev).unwrap();
    let e = run(&list, "LAB_NOSY_GO", &Value::Null).unwrap_err();
    assert!(e.contains("reads.editor"), "{e}");
    assert!(host.unload("lab.nosy"));

    // Reading is not editing.
    let p = package(
        &tmp.path().join("b"),
        "reads = [\"editor\"]",
        "",
        "local ed = host.editor()\nhost.edit_editor({ changes = { { from = 0, to = 0, insert = \"x\" } } })",
    );
    host.load(&p, Origin::Dev).unwrap();
    let e = run(&list, "LAB_NOSY_GO", &Value::Null).unwrap_err();
    assert!(e.contains("writes.editor"), "{e}");
    assert_eq!(editor.text(), "- a");
    assert!(host.unload("lab.nosy"));

    // A key in the editor needs writes = ["editor"] at load.
    let e = try_package(
        &tmp.path().join("c"),
        "reads = [\"editor\"]",
        "[[keymaps]]\nkey = \"Tab\"\naction = \"LAB_NOSY_GO\"\n",
        "",
    )
    .unwrap_err();
    assert!(e.contains("writes = [\"editor\"]"), "{e}");

    // A change that does not fit the text is refused, not applied; a widget
    // must name its own embed or view.
    let p = package(
        &tmp.path().join("d"),
        "writes = [\"editor\"]",
        "",
        "local which = args.which\n\
         if which == 1 then host.edit_editor({ changes = { { from = 0, to = 99, insert = \"\" } } }) end\n\
         if which == 2 then host.decorate_editor({ { kind = \"widget\", at = 0, view = \"elsewhere\" } }) end\n\
         if which == 3 then host.edit_editor({ selection = { { anchor = 1, head = 1 } }, base = 41 }) end",
    );
    host.load(&p, Origin::Dev).unwrap();
    let e = run(&list, "LAB_NOSY_GO", &json!({ "which": 1 })).unwrap_err();
    assert!(e.contains("past the end"), "{e}");
    let e = run(&list, "LAB_NOSY_GO", &json!({ "which": 2 })).unwrap_err();
    assert!(e.contains("not one of its own"), "{e}");
    let e = run(&list, "LAB_NOSY_GO", &json!({ "which": 3 })).unwrap_err();
    assert!(e.contains("read it again"), "{e}");
    assert_eq!(editor.text(), "- a");
    assert_eq!(selection(&editor), (3, 3));
    let _ = SelRange::caret(0);
}
