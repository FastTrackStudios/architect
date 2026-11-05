//! The note editor's fence registry is the extensions' (#180): the
//! Vault's "A chart in a note" in its Edit mode keeps its ```` ```kf ````
//! fence as source and mounts the Board's chart under it (the Board's
//! `[[embeds]] kf`, a page drawn in a sub-document); the ```` ```rust ````
//! fence nobody claims gets nothing. Turned off, the Board's chart goes
//! from the editor too.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    // The node type is blitz-dom's, which this crate does not name.
    clippy::redundant_closure_for_method_calls
)]

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
            .map(|n| n.outer_html_pretty())
            .collect::<Vec<_>>()
            .join("\n");
    }
}

fn play(name: &str, steps: &str) -> String {
    let source = format!(
        "id note-fence-{name}\ntitle \"{name}\"\nview {{width 1280, height 800}}\n\
         capture {{fps 10, width 640}}\nsteps (\n{steps}    {{\n        id end\n        hold 0.3\n    }}\n)\n"
    );
    let scenario = architect_guide::load(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut film = EditorHtml::default();
    if let Err(e) =
        architect_guide::run::run(&architect_lab_media::app(), &scenario, Some(&mut film))
    {
        panic!("{name}: {e}\n{}", film.0);
    }
    film.0
}

/// The chart note open in the Vault, in its Edit mode, the Board's chart
/// mounted under its fence.
const OPEN: &str = r##"    {id vault, do @run{id LAB_GO_VAULT}, done {check ({id LAB_STATE, has {tab vault}})}}
    {id open, do @run{id WORKSPACE_OPEN, args {view note, id note-chart, title "A chart in a note", at {replace {pane note-welcome}}}}}
    {id focus, do @run{id WORKSPACE_FOCUS, args {pane note-chart}}, done {check ({id WORKSPACE_LAYOUT, has {active note-chart}}), timeout 5}}
    {id edit, target @css"#note-edit", do @click{}}
    {id mounted, target @css"[data-note-editor=\"note-chart\"]", done {check ({id LAB_EDITOR_STATE, has {context note}}), timeout 5}}
    {id chart, target @css"[data-editor-mount] [data-embed=\"lab.board/kf\"]", done {timeout 10}}
"##;

const BOARD_OFF: &str = r"    {id off, do @run{id EXT_DISABLE, args {id lab.board}}}
    {id settle, hold 0.5}
";

#[test]
fn the_note_editor_mounts_a_claimed_fences_embed_and_drops_it_with_its_extension() {
    let folder = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary sets it, before the lab (and
    // its threads) start.
    unsafe { std::env::set_var("ARCHITECT_LAB_CONFIG_DIR", folder.path()) };

    let on = play("on", OPEN);
    assert!(
        on.contains("data-editor-mount=\"fence-0\"") && on.contains("data-fence=\"kf\""),
        "the kf fence's mount: {on}"
    );
    assert!(
        on.contains("data-embed=\"lab.board/kf\""),
        "the Board's chart in it: {on}"
    );
    assert!(
        !on.contains("data-editor-mount=\"fence-1\""),
        "the rust fence is nobody's: {on}"
    );
    // Its source is still the note's text.
    assert!(on.contains("[Intro]"), "{on}");

    let off = play("off", &format!("{OPEN}{BOARD_OFF}"));
    assert!(
        !off.contains("data-embed=\"lab.board/kf\""),
        "the chart went with the Board: {off}"
    );
    assert!(off.contains("[Intro]"), "the fence stays: {off}");
}
