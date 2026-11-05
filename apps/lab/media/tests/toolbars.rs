//! An extension's toolbars as the lab draws them (#307): the metronome's
//! in the right dock and in a note's header, its click a toggle whose
//! buttons show it on after one press and off after another, and the
//! dock's armed by a right-click with its group's dropdown open. Each state's
//! toolbars, as HTML, match `tests/golden/toolbars-<state>.html`;
//! `ARCHITECT_GOLDEN=record` writes them again.
//!
//! Each run is on a lab folder of its own, empty
//! (`ARCHITECT_LAB_CONFIG_DIR`); one test plays them in turn, as the
//! folder is the process's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use architect_guide::run::{Desktop, Film, Pointer};

/// Every toolbar in the main window, as HTML, at the last frame shown.
#[derive(Default)]
struct Toolbars(String);

impl Film for Toolbars {
    fn frame(&mut self, desktop: &Desktop, _: Pointer) {
        let Some(main) = desktop.windows().first() else {
            return;
        };
        let doc = main.doc.inner.borrow();
        let Ok(found) = doc.query_selector_all("[data-toolbar]") else {
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

/// The metronome on, the Vault's welcome note in front, the toolbars
/// there.
const ON_SHOW: &str = r#"    {id open, do @run{id APP_SETTINGS}}
    {id workspace, target @text"Workspace", do @click{}}
    {id load, target @css"[id=\"setting-workspace.metronome\"] [role=\"switch\"]", do @click{}}
    {id loaded, target @text"Metronome", done {timeout 20}}
    {id close, target @css"[aria-label=\"Close settings\"]", do @click{}}
    {id vault, do @run{id LAB_GO_VAULT}, done {on (@changed"lab"), check ({id LAB_STATE, has {tab vault}})}}
    {id welcome, target @tab"note-welcome", do @click{}, done {check ({id WORKSPACE_LAYOUT, has {active note-welcome}})}}
    {id docked, target @css"[data-toolbar=\"fts.metronome/transport\"]", done {timeout 20}}
"#;

/// The `n`th press of the note header's click button (from 0): the click
/// on after an odd count, off after an even one.
fn press(n: usize) -> String {
    let on = n.is_multiple_of(2);
    format!(
        r#"    {{
        id press-{n}
        target @css"[data-toolbar=\"fts.metronome/note\"] [data-tool=\"FTS_METRONOME_CLICK\"]"
        do @click{{}}
        done {{check ({{id FTS_METRONOME_STATUS, has {{click {on}}}}})}}
    }}
"#
    )
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("toolbars-{name}.html"))
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

/// Plays the metronome on and `presses` presses of its click, and
/// compares the toolbars at the end with `golden/toolbars-<name>.html`.
fn toolbars_look(name: &str, presses: usize, more: &str) -> String {
    let mut steps = ON_SHOW.to_owned();
    for n in 0..presses {
        steps.push_str(&press(n));
    }
    steps.push_str(more);
    let source = format!(
        "id toolbars-{name}\ntitle \"{name}\"\nview {{width 1440, height 860}}\n\
         capture {{fps 10, width 960}}\nsteps (\n{steps}    {{\n        id end\n        hold 0.5\n    }}\n)\n"
    );
    let scenario = architect_guide::load(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
    let folder = tempfile::tempdir().unwrap();
    // SAFETY: the only test in this binary sets it, between runs, before
    // the lab (and its threads) start.
    unsafe { std::env::set_var("ARCHITECT_LAB_CONFIG_DIR", folder.path()) };
    let mut film = Toolbars::default();
    architect_guide::run::run(&architect_lab_media::app(), &scenario, Some(&mut film))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    let path = golden(name);
    if std::env::var("ARCHITECT_GOLDEN").as_deref() == Ok("record") {
        std::fs::write(&path, &film.0).unwrap();
        return film.0;
    }
    let want = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{}: {e} (ARCHITECT_GOLDEN=record)", path.display()));
    if want != film.0 {
        let got = path.with_extension("got.html");
        std::fs::write(&got, &film.0).unwrap();
        panic!(
            "toolbars-{name} does not look the same (diff {} {})",
            path.display(),
            got.display()
        );
    }
    film.0
}

/// The dock's click armed (its right-click), and its group's dropdown
/// open: a heading, the click ticked, a separator, a submenu.
const ARM_AND_OPEN: &str = r#"    {
        id arm
        target @css"[data-toolbar=\"fts.metronome/transport\"] [data-tool=\"FTS_METRONOME_CLICK\"]"
        do @click{secondary true}
        done {check ({id TOOLBAR_LIST, has {armed FTS_METRONOME_CLICK}})}
    }
    {
        id more
        target @css"[data-toolbar=\"fts.metronome/transport\"] [data-tool=\"menu:More\"]"
        do @click{}
    }
    {id opened, target @css"[data-toolbar=\"fts.metronome/transport\"] [role=\"menu\"]", done {timeout 5}}
"#;

/// A press outside the open dropdown (in another pane) closes it.
const PRESS_AWAY: &str = r#"    {
        id away
        target @text"Every note and its links"
        do @click{}
        hold 0.3
    }
"#;

/// Escape, with the dropdown's button focused, closes it.
const ESCAPE: &str = r#"    {
        id escape
        target @css"[data-toolbar=\"fts.metronome/transport\"] [data-tool=\"menu:More\"]"
        do @key{key Escape}
        hold 0.3
    }
"#;

/// A dropdown closes on a press outside it and on Escape (part of the
/// test below: one test in this binary sets the lab's folder).
fn a_dropdown_closes_on_a_press_outside_it_and_on_escape() {
    for (name, how) in [("menu-away", PRESS_AWAY), ("menu-escape", ESCAPE)] {
        let html = toolbars_look(name, 1, &format!("{ARM_AND_OPEN}{how}"));
        assert!(html.contains(r#"data-armed="true""#), "{name}: {html}");
        assert!(
            !html.contains(r#"role="menu""#),
            "{name}: still open: {html}"
        );
        assert!(!html.contains("data-menu-backdrop"), "{name}: {html}");
    }
}

#[test]
fn a_toggles_buttons_show_it_on_then_off_in_a_dock_and_a_pane_header() {
    let on = toolbars_look("on", 1, "");
    let off = toolbars_look("off", 2, "");
    let armed = toolbars_look("armed-menu", 1, ARM_AND_OPEN);
    // Armed: the dock's click says so; the dropdown's click row is ticked.
    assert!(armed.contains(r#"data-armed="true""#), "{armed}");
    assert!(armed.contains(r#"aria-checked="true""#), "{armed}");
    assert!(armed.contains("Status"), "the submenu: {armed}");
    // Both toolbars, the dock's and the note header's, each time.
    for html in [&on, &off] {
        assert!(html.contains(r#"data-toolbar="fts.metronome/transport""#));
        assert!(html.contains(r#"data-toolbar="fts.metronome/note""#));
    }
    // The click's buttons (one on each) are pressed, then not.
    assert_eq!(on.matches(r#"aria-pressed="true""#).count(), 2, "{on}");
    assert_eq!(on.matches(r#"aria-pressed="false""#).count(), 0, "{on}");
    assert_eq!(off.matches(r#"aria-pressed="false""#).count(), 2, "{off}");
    assert_eq!(off.matches(r#"aria-pressed="true""#).count(), 0, "{off}");
    a_dropdown_closes_on_a_press_outside_it_and_on_escape();
}
