//! A distribution as code: its identity, its compiled-in list, what it
//! describes, the folder it adopts.

use architect_distribution::{Bundle, Distribution, architect, copy_once, slug};

#[test]
fn a_new_distribution_takes_its_folder_app_and_bundle_id_from_its_name() {
    let d = Distribution::new("My Studio!");
    assert_eq!(slug("My Studio!"), "my-studio");
    assert_eq!(d.name(), "My Studio!");
    assert_eq!(d.folder_name(), "my-studio");
    assert_eq!(d.app_name(), "my-studio");
    assert_eq!(d.id(), "app.my-studio");
    assert!(d.extensions().is_empty(), "nothing compiled in");
    assert!(!d.is_sandbox());
}

#[test]
fn the_architect_app_is_a_distribution_of_its_first_party_extensions() {
    let d = architect();
    assert_eq!(d.name(), "Architect");
    assert_eq!(d.folder_name(), "architect");
    assert_eq!(d.app_name(), "architect-desktop");
    assert_eq!(d.id(), "app.fasttrackstudio.architect");
    let ids: Vec<String> = d.describe()["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_owned())
        .collect();
    for want in ["projects", "tasks", "time", "calendar", "dayplan"] {
        assert!(ids.iter().any(|id| id == want), "{want} in {ids:?}");
    }
    assert_eq!(ids.len(), 8, "{ids:?}");
}

#[test]
fn a_renamed_build_keeps_what_is_compiled_in_and_takes_an_identity_of_its_own() {
    let d = architect()
        .named("Studio")
        .bundle_id("app.fasttrackstudio.studio");
    assert_eq!(d.name(), "Studio");
    assert_eq!(d.folder_name(), "studio");
    assert_eq!(d.app_name(), "studio");
    assert_eq!(d.id(), "app.fasttrackstudio.studio");
    assert_eq!(d.extensions().len(), architect().extensions().len());
}

#[test]
fn an_extension_both_linked_and_in_a_bundle_is_compiled_in_once() {
    let d = Distribution::new("Twice")
        .linked(&architect_tasks::Tasks)
        .bundle(
            Bundle::new("pm", "PM")
                .member(&architect_tasks::Tasks)
                .member(&architect_projects::Projects),
        );
    assert_eq!(d.extensions().len(), 2);
    let described = d.describe();
    let tasks = described["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "tasks")
        .unwrap();
    assert_eq!(tasks["bundle"], "pm", "it says which bundle it is in");
    assert_eq!(described["bundles"][0]["members"][1], "projects");
}

#[test]
fn its_own_block_renderers_come_first_and_extensions_without_pages_add_none() {
    let d = architect().block(architect_sites::FnBlock::new("kf", &["kf"], |b| {
        Ok(architect_sites::Drawn::Html(format!(
            "<svg>{}</svg>",
            b.source
        )))
    }));
    let tmp = tempfile::tempdir().unwrap();
    let blocks = d.blocks(tmp.path());
    assert!(!blocks.is_empty());
}

#[test]
fn the_old_folder_is_copied_into_an_empty_app_folder_once() {
    let tmp = tempfile::tempdir().unwrap();
    let (lab, app) = (
        tmp.path().join("architect-lab"),
        tmp.path().join("architect"),
    );
    std::fs::create_dir_all(lab.join("themes")).unwrap();
    std::fs::write(lab.join("settings.json"), "{}").unwrap();
    std::fs::write(lab.join("themes/mine.json"), "{\"id\":\"mine\"}").unwrap();
    std::fs::create_dir_all(&app).unwrap();
    assert!(
        copy_once(&lab, &app).unwrap(),
        "an empty folder takes the old one's"
    );
    assert_eq!(
        std::fs::read_to_string(app.join("settings.json")).unwrap(),
        "{}"
    );
    assert!(app.join("themes/mine.json").is_file());
    assert!(
        lab.join("settings.json").is_file(),
        "the old folder is left"
    );
    std::fs::write(lab.join("settings.json"), "{\"changed\":true}").unwrap();
    assert!(
        !copy_once(&lab, &app).unwrap(),
        "only once: the app's folder has files now"
    );
    assert_eq!(
        std::fs::read_to_string(app.join("settings.json")).unwrap(),
        "{}"
    );
}

#[test]
fn no_old_folder_means_nothing_to_copy() {
    let tmp = tempfile::tempdir().unwrap();
    let app = tmp.path().join("architect");
    assert!(!copy_once(&tmp.path().join("architect-lab"), &app).unwrap());
    assert!(!app.exists());
}

/// A linked extension whose embed page draws ```chord fences.
struct Chord;

impl architect_extensions::linked::Extension for Chord {
    fn manifest(&self) -> &'static str {
        include_str!("chord/extension.toml")
    }

    fn files(&self) -> &'static [(&'static str, &'static str)] {
        &[("pages/chord.tsx", include_str!("chord/pages/chord.tsx"))]
    }

    fn build(&self, _: &mut architect_extensions::linked::Builder) {}
}

#[test]
fn a_compiled_in_extensions_embed_page_draws_its_fence_on_a_site() {
    let vault = tempfile::tempdir().unwrap();
    std::fs::write(
        vault.path().join("song.md"),
        "# Song\n\n```chord\nAm7\n```\n",
    )
    .unwrap();
    let work = tempfile::tempdir().unwrap();
    let blocks = Distribution::new("Chords")
        .linked(&Chord)
        .blocks(&work.path().join("linked"));
    assert!(
        work.path()
            .join("linked/test.chord/pages/chord.tsx")
            .is_file(),
        "its package is written for the page runner"
    );
    let out = work.path().join("site");
    architect_sites::build(
        &architect_sites::Options::new(vault.path(), ""),
        &blocks,
        &out,
    )
    .unwrap();
    let page = std::fs::read_to_string(out.join("song/index.html")).unwrap();
    assert!(page.contains("chord Am7"), "drawn by the page: {page}");
}
