//! The toolbar editor (#313), as a person uses it in the lab: Edit
//! toolbars, a new toolbar from nothing, an action (found by the palette's
//! search), a second, a dropdown with a row of its own, the Board's tempo
//! widget (picked by its extension, previewed live); the two buttons
//! grouped from an item's context menu, the dropdown dragged into the
//! group, a drag cancelled with Escape, the whole group dragged past the
//! widget; a button restyled (label, image icon, fill), the toolbar's
//! corners and colour preset. Done, it is written; the lab started again
//! on the same folder shows it the same.
//!
//! The toolbar, as HTML, matches `tests/golden/toolbar-editor-<state>.html`
//! (`built`: after Done; `menu`: its dropdown open at the window's edge,
//! moved left to stay in it; `editing`: in edit mode again, after the
//! restart);
//! `ARCHITECT_GOLDEN=record` writes them again. One test plays the runs in
//! turn, as the lab's folder (`ARCHITECT_LAB_CONFIG_DIR`) is the process's.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use architect_guide::run::{Desktop, Film, Pointer};

const KEY: &str = "user/toolbar";

/// The person's toolbar, as HTML, at the last frame shown.
#[derive(Default)]
struct Toolbar(String);

impl Film for Toolbar {
    fn frame(&mut self, desktop: &Desktop, _: Pointer) {
        let Some(main) = desktop.windows().first() else {
            return;
        };
        let doc = main.doc.inner.borrow();
        let Ok(Some(found)) = doc.query_selector(&format!("[data-toolbar=\"{KEY}\"]")) else {
            return;
        };
        if let Some(n) = doc.get_node(found) {
            self.0 = without_ids(&n.outer_html_pretty());
        }
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

fn css(selector: &str) -> String {
    format!("@css\"{}\"", selector.replace('"', "\\\""))
}

fn item(at: &str) -> String {
    css(&format!("[data-edit-item=\"{KEY}#{at}\"]"))
}

/// `TOOLBAR_GET`'s toolbar has `has`.
fn holds(has: &str) -> String {
    format!("done {{check ({{id TOOLBAR_GET, args {{id \"{KEY}\"}}, has {{toolbar {has}}}}})}}")
}

/// Picks action `id` (searched as `search`) for whatever asked.
fn pick(n: &str, search: &str, id: &str) -> String {
    format!(
        "    {{id search-{n}, target {}, do @type{{text \"{search}\"}}}}\n    {{id pick-{n}, target {}, do @click{{}}}}\n",
        css("[data-testid=\"toolbar-action-search\"]"),
        css(&format!("[data-pick=\"{id}\"]")),
    )
}

/// The + chip at `at`, then its menu's `what`.
fn insert(n: &str, at: usize, what: &str) -> String {
    format!(
        "    {{id plus-{n}, target {}, do @click{{}}}}\n    {{id insert-{n}, target {}, do @click{{}}}}\n",
        css(&format!("[data-insert-at=\"{KEY}#{at}\"]")),
        css(&format!("[data-insert=\"{what}\"]")),
    )
}

/// Building it, from Edit toolbars to Done.
fn build() -> String {
    let mut s = String::new();
    s.push_str("    {id edit, do @run{id TOOLBAR_EDIT}}\n");
    s.push_str(&format!(
        "    {{id bar, target {}, done {{timeout 5}}}}\n",
        css("[data-testid=\"toolbar-edit-bar\"]")
    ));
    s.push_str(&format!(
        "    {{id new, target {}, do @click{{}}}}\n",
        css("[data-testid=\"toolbar-new\"]")
    ));
    s.push_str(&format!("    {{id made, {}}}\n", holds("{title Toolbar}")));
    s.push_str(&format!(
        "    {{id shown, target {}, done {{timeout 20}}}}\n",
        css(&format!("[data-toolbar=\"{KEY}\"]"))
    ));
    // Its settings opened with it: closed, for now.
    s.push_str(&format!(
        "    {{id close-new, target {}, do @click{{}}}}\n",
        css("[aria-label=\"Close the settings\"]")
    ));
    // Two actions.
    s.push_str(&insert("a", 0, "action"));
    s.push_str(&pick("a", "pin to", "LAB_BOARD_PIN"));
    s.push_str(&format!(
        "    {{id has-a, {}}}\n",
        holds("{items ({action LAB_BOARD_PIN})}")
    ));
    s.push_str(&insert("b", 1, "action"));
    s.push_str(&pick("b", "clear the", "LAB_BOARD_CLEAR"));
    // A dropdown: its settings open; a row of its own.
    s.push_str(&insert("c", 2, "dropdown"));
    s.push_str(&format!(
        "    {{id add-row, target {}, do @click{{}}}}\n",
        css("[data-testid=\"toolbar-menu-add-action\"]")
    ));
    s.push_str(&pick("c", "the board", "LAB_BOARD_STATE"));
    s.push_str(&format!(
        "    {{id close-c, target {}, do @click{{}}}}\n",
        css("[aria-label=\"Close the settings\"]")
    ));
    // The tempo widget, previewed first.
    s.push_str(&insert("d", 3, "widget"));
    s.push_str(&format!(
        "    {{id board, target {}, do @click{{}}}}\n",
        css("[data-category=\"lab.board\"]")
    ));
    s.push_str(&format!(
        "    {{id tempo, target {}, do @click{{}}}}\n",
        css("[data-widget-pick=\"lab.board/tempo\"]")
    ));
    s.push_str(&format!(
        "    {{id preview, target {}, done {{timeout 5}}}}\n",
        css("[data-testid=\"toolbar-widget-preview\"] [data-testid=\"board-tempo\"]")
    ));
    s.push_str(&format!(
        "    {{id put, target {}, do @click{{}}, {}}}\n",
        css("[data-testid=\"toolbar-widget-insert\"]"),
        holds("{items ({action LAB_BOARD_PIN} {action LAB_BOARD_CLEAR} {label Menu, menu ({kind heading, label Menu} {action LAB_BOARD_STATE})} {kind widget, view \"lab.board/tempo\"})}")
    ));
    // The two buttons as one group, from the first's context menu.
    s.push_str(&format!(
        "    {{id menu, target {}, do @click{{secondary true}}}}\n",
        item("0")
    ));
    s.push_str(&format!(
        "    {{id group, target @text\"Group with the next\", do @click{{}}, {}}}\n",
        holds("{items ({kind group} {label Menu} {kind widget})}")
    ));
    // The dropdown dragged into the group, at its end.
    s.push_str(&format!(
        "    {{id into, target {}, do @drag{{to {}, edge right}}, {}}}\n",
        item("1"),
        item("0.1"),
        holds("{items ({kind group, items ({action LAB_BOARD_PIN} {action LAB_BOARD_CLEAR} {label Menu})} {kind widget})}")
    ));
    // A drag let go with Escape: nothing moves.
    s.push_str(&format!(
        "    {{id cancelled, target {}, do @drag{{to {}, key Escape}}, {}}}\n",
        item("1"),
        item("0.0"),
        holds("{items ({kind group} {kind widget})}")
    ));
    // The whole group, by its grip, past the widget.
    s.push_str(&format!(
        "    {{id regroup, target {}, do @drag{{to {}, edge right}}, {}}}\n",
        css(&format!("[data-edit-item=\"{KEY}#0\"] [data-group-grip=\"0\"]")),
        item("1"),
        holds("{items ({kind widget} {kind group})}")
    ));
    // Restyled: the pin's label, image and fill.
    s.push_str(&format!(
        "    {{id select, target {}, do @click{{}}}}\n",
        item("1.0")
    ));
    for (n, field, text) in [
        ("label", "toolbar-item-label", "Pin"),
        ("image", "toolbar-item-image", "icons/star.svg"),
    ] {
        s.push_str(&format!(
            "    {{id {n}, target {}, do @click{{}}}}\n    {{id type-{n}, do @type{{text \"{text}\"}}}}\n",
            css(&format!("[data-testid=\"{field}\"]"))
        ));
    }
    s.push_str(&format!(
        "    {{id fill, target {}, do @click{{}}, {}}}\n",
        css("[data-testid=\"toolbar-item-fill\"] [data-token=\"primary\"]"),
        holds("{items ({kind widget} {kind group, items ({label Pin, image \"icons/star.svg\", colors {fill primary}} {action LAB_BOARD_CLEAR} {label Menu})})}")
    ));
    // The toolbar's own: rounder, the soft preset.
    s.push_str(&format!(
        "    {{id settings, target {}, do @click{{}}}}\n",
        css(&format!("[data-toolbar-settings=\"{KEY}\"]"))
    ));
    for n in 0..2 {
        s.push_str(&format!(
            "    {{id round-{n}, target {}, do @click{{}}}}\n",
            css("[data-testid=\"toolbar-rounding\"] [data-step=\"up\"]")
        ));
    }
    s.push_str(&format!(
        "    {{id preset, target {}, do @click{{}}, {}}}\n",
        css("[data-testid=\"toolbar-preset\"] [data-value=\"soft\"]"),
        holds("{style {rounding 10, preset soft}}")
    ));
    s.push_str(&format!(
        "    {{id done, target {}, do @click{{}}, {}}}\n",
        css("[data-testid=\"toolbar-edit-done\"]"),
        holds("{title Toolbar}")
    ));
    s
}

/// Waiting for it after a restart.
fn reload() -> String {
    format!(
        "    {{id back, target {}, done {{timeout 20}}}}\n",
        css(&format!("[data-toolbar=\"{KEY}\"] [data-widget]"))
    )
}

/// In edit mode again: its chips, its items' outlines, the gear.
fn editing() -> String {
    let mut s = reload();
    s.push_str("    {id edit, do @run{id TOOLBAR_EDIT}}\n");
    s.push_str(&format!(
        "    {{id chips, target {}, done {{timeout 5}}}}\n",
        css(&format!("[data-insert-at=\"{KEY}#2\"]"))
    ));
    s
}

/// Its dropdown opened, near the window's right edge: kept in it.
fn menu() -> String {
    let mut s = reload();
    s.push_str(&format!(
        "    {{id open, target {}, do @click{{}}}}\n    {{id opened, target {}, done {{timeout 5}}}}\n",
        css(&format!("[data-toolbar=\"{KEY}\"] [data-tool=\"menu:Menu\"]")),
        css(&format!("[data-toolbar=\"{KEY}\"] [role=\"menu\"][data-kept]")),
    ));
    s
}

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden")
        .join(format!("toolbar-editor-{name}.html"))
}

/// The Vault on show: the workspace, where toolbars are.
const VAULT: &str = "    {id vault, do @run{id LAB_GO_VAULT}, done {on (@changed\"lab\"), check ({id LAB_STATE, has {tab vault}})}}\n";

/// Plays `steps` on the lab with its folder at `folder`: the toolbar at
/// the end.
fn play(name: &str, folder: &Path, steps: &str) -> String {
    let steps = format!("{VAULT}{steps}");
    let source = format!(
        "id toolbar-editor-{name}\ntitle \"{name}\"\nview {{width 1440, height 860}}\n\
         capture {{fps 10, width 960}}\nsteps (\n{steps}    {{\n        id end\n        hold 0.5\n    }}\n)\n"
    );
    let scenario = architect_guide::load(&source).unwrap_or_else(|e| panic!("{name}: {e}\n{source}"));
    // SAFETY: the only test in this binary sets it, between runs, before
    // the lab (and its threads) start.
    unsafe { std::env::set_var("ARCHITECT_LAB_CONFIG_DIR", folder) };
    let mut film = Toolbar::default();
    architect_guide::run::run(&architect_lab_media::app(), &scenario, Some(&mut film))
        .unwrap_or_else(|e| panic!("{name}: {e}"));
    film.0
}

fn matches_golden(name: &str, html: &str) {
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
            "toolbar-editor-{name} does not look the same (diff {} {})",
            path.display(),
            got.display()
        );
    }
}

/// A star, the pin's image icon.
const STAR: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M12 2l3 7h7l-5.5 4.5L18 21l-6-4-6 4 1.5-7.5L2 9h7z" fill="currentColor"/></svg>"#;

#[test]
fn a_toolbar_built_from_empty_in_the_editor_survives_a_restart() {
    let folder = tempfile::tempdir().unwrap();
    let icons = folder.path().join("toolbars/icons");
    std::fs::create_dir_all(&icons).unwrap();
    std::fs::write(icons.join("star.svg"), STAR).unwrap();

    let built = play("built", folder.path(), &build());
    // Written: the file, and a backup of each write before it.
    let file = folder.path().join("toolbars/user-toolbar.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("lab.board/tempo") && text.contains("icons/star.svg"), "{text}");
    assert!(folder.path().join("toolbars/.backups/user-toolbar.1.toml").exists());
    // Drawn as built: the widget, then the group (its pin's image, its fill).
    assert!(built.contains(r#"data-widget="lab.board/tempo""#), "{built}");
    assert!(built.contains("data:image/svg+xml;base64,"), "{built}");
    assert!(built.contains("var(--primary"), "{built}");
    assert!(!built.contains("data-insert-at"), "not editing: {built}");
    matches_golden("built", &built);

    // The lab again, on the same folder: the same toolbar.
    let again = play("reload", folder.path(), &reload());
    assert_eq!(again, built, "the same after a restart");

    // Its dropdown, at the right dock's edge: moved left to stay in the
    // window, not clipped (#315's leftover).
    let opened = play("menu", folder.path(), &menu());
    assert!(opened.contains(r#"data-kept="left "#), "{opened}");
    matches_golden("menu", &opened);

    let editing = play("editing", folder.path(), &editing());
    assert!(editing.contains(r#"data-editing="true""#), "{editing}");
    assert_eq!(editing.matches("data-insert-at=").count(), 3, "a chip before each item and one at the end: {editing}");
    matches_golden("editing", &editing);
}
