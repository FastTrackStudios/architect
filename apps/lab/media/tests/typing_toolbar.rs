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
    architect_guide::run::run(&architect_lab_media::app(), &scenario, Some(&mut film))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
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
const CAPTURE: &str = r#"    {id capture, do @run{id LAB_CAPTURE}, done {check ({id LAB_STATE, has {page capture, bar editing}})}}
"#;

#[test]
fn the_typing_toolbar_on_a_phone() {
    // The editor's own items, as the bar showed them before it was a
    // toolbar.
    let folder = tempfile::tempdir().unwrap();
    let builtins = play("builtins", folder.path(), CAPTURE);
    matches("builtins", &builtins);
    assert!(builtins.contains(r#"data-bar-item="LAB_CAPTURE_TAG""#), "{builtins}");
    assert!(builtins.contains(r#"data-bar-item="hide-keyboard""#), "{builtins}");
}
