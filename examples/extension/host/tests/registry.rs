//! The example's Luau extension, end to end against its registry: found,
//! installed, run, its settings on the page, its overlay drawing over the
//! timeline with a tool window, reloaded on save, and gone with every
//! part when it is removed.
//!
//! The registry is a git repository made of the example's registry folder
//! (`examples/extension/registry`), as one published would be.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::float_cmp
)]

use std::path::{Path, PathBuf};
use std::process::Command;

use architect::action::{ActionCaller, ActionList};
use architect_canvas::imgui::{Input, WindowPointer};
use architect_canvas::luau::Session;
use architect_canvas::{Lane, Plane, Point, Shape, Size, Spaces, TimeAxis};
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Event, SharedSections};
use architect_extensions::install::{FolderRegistry, GitRegistry};
use architect_extensions::process::Processes;
use example_extension_host::{HELLO, Overlay, TIMELINE, extensions, overlays};
use futures::executor::block_on;
use serde_json::{Value, json};

const GREET: &str = "EXAMPLE_HELLO_GREET";
const WINDOW: &str = "Hello/Ruler";

/// The example's registry folder.
fn example_registry() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../registry")
}

fn copy(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let dest = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy(&e.path(), &dest);
        } else {
            std::fs::copy(e.path(), dest).unwrap();
        }
    }
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=example",
            "-c",
            "user.email=example@example.com",
        ])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .status()
        .unwrap()
        .success();
    assert!(ok, "git {args:?}");
}

/// The example's registry, published: a git repository of the folder.
fn git_registry(tmp: &Path) -> PathBuf {
    let repo = tmp.join("registry");
    copy(&example_registry(), &repo);
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["add", "."]);
    git(
        &repo,
        &["commit", "--quiet", "--no-verify", "-m", "registry"],
    );
    repo
}

/// The keys of Hello's settings on the page.
fn hello_keys(sections: &SharedSections) -> Vec<String> {
    sections
        .registry()
        .section("hello")
        .map(|s| s.settings().map(|t| t.key.clone()).collect())
        .unwrap_or_default()
}

fn run(list: &ActionList, id: &str, args: &Value) -> Result<Value, String> {
    block_on(list.run_json_async(id, args.to_string().as_bytes()))
        .map(|out| serde_json::from_slice(&out).unwrap_or(Value::Null))
        .map_err(|e| e.to_string())
}

/// The timeline, 400 points wide at 40 a second: ten seconds of it.
fn timeline() -> Spaces {
    Spaces::sized(400.0, 120.0)
        .with_time(TimeAxis {
            start: 0.0,
            pixels_per_second: 40.0,
        })
        .with_lane(Lane::new("a", "Lane A", 24.0, 48.0))
}

/// One frame of input: the tool window's size, the pointer in it.
fn input(pointer: Option<Point>, down: bool) -> Input {
    Input {
        pointer: pointer.map(|at| WindowPointer {
            window: WINDOW.into(),
            at,
        }),
        down,
        sizes: [(WINDOW.to_owned(), Size::new(280.0, 160.0))].into(),
        ..Input::default()
    }
}

/// The overlay running, the timeline shown, one frame drawn.
fn start(overlay: &Overlay, list: &ActionList) -> Session {
    let (report, session) = block_on(Session::start(
        &overlay.runtime(list.clone()),
        &ActionCaller::app(),
        &overlay.source,
        Value::Null,
        &overlay.manifest.name,
    ));
    assert!(report.result.is_ok(), "{report:?}");
    let mut session = session.expect("it runs");
    session.publish(TIMELINE, Some(timeline()));
    block_on(session.frame(input(None, false)));
    session
}

/// The x of each tick the ruler drew.
fn ticks(session: &Session) -> Vec<f32> {
    let out = session.output();
    let layer = out
        .layers_on(TIMELINE, Plane::Over)
        .next()
        .expect("a layer");
    layer
        .draw
        .shapes()
        .iter()
        .filter_map(|s| match s {
            Shape::Line { from, .. } => Some(from.x),
            _ => None,
        })
        .collect()
}

/// The texts in the tool window, and where each is.
fn texts(session: &Session) -> Vec<(String, Point)> {
    session
        .output()
        .window(WINDOW)
        .expect("the tool window")
        .draw
        .shapes()
        .iter()
        .filter_map(|s| match s {
            Shape::Text { text, at, .. } => Some((text.clone(), *at)),
            _ => None,
        })
        .collect()
}

fn app(
    tmp: &Path,
    source: impl architect_extensions::install::Source + 'static,
) -> (ActionList, SharedSections, Extensions) {
    let (list, sections) = (ActionList::new(), SharedSections::default());
    let ext = extensions(
        list.clone(),
        sections.clone(),
        Processes::default(),
        tmp.join("installed"),
        source,
    );
    (list, sections, ext)
}

#[test]
fn hello_installs_from_the_registry_draws_reloads_on_save_and_goes() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = git_registry(tmp.path());
    let (list, sections, ext) = app(
        tmp.path(),
        GitRegistry::new(repo.display().to_string(), tmp.path().join("checkout")),
    );
    assert!(ext.refresh().is_empty(), "the registry is cloned");

    // In the registry, not loaded: nothing of it in the app.
    let found = ext.search("hello");
    assert_eq!(found[0].hit.listing.id, HELLO);
    assert_eq!(found[0].source, "Example registry");
    assert!(hello_keys(&sections).is_empty());
    assert!(list.info(GREET).is_none());
    assert!(overlays(ext.host(), TIMELINE).is_empty());

    // Installed as ⌘K or a model would (EXT_INSTALL): loaded at once.
    let installed = run(&list, "EXT_INSTALL", &json!({ "id": HELLO })).unwrap();
    assert_eq!(installed["loaded"], true);
    assert_eq!(
        hello_keys(&sections),
        ["example.hello.greeting", "example.hello.loud"]
    );
    assert_eq!(
        run(&list, GREET, &json!({ "name": "reader" })).unwrap(),
        "Hello, reader!"
    );
    assert_eq!(list.info(GREET).unwrap().category, "Extensions");

    // Its overlay: a tick every two seconds across the timeline, labelled,
    // and a tool window.
    let [overlay] = overlays(ext.host(), TIMELINE).try_into().unwrap();
    let mut ruler = start(&overlay, &list);
    assert_eq!(ticks(&ruler), [0.0, 80.0, 160.0, 240.0, 320.0, 400.0]);
    let labels: Vec<String> = texts(&ruler).into_iter().map(|(t, _)| t).collect();
    assert_eq!(labels[..4], ["2 s", "Every", "Labels", "Greet"]);

    // Its Greet button runs its own action, through the app's list.
    let greet = texts(&ruler)
        .into_iter()
        .find(|(t, _)| t == "Greet")
        .map(|(_, at)| Point::new(at.x + 4.0, at.y + 4.0))
        .unwrap();
    for down in [false, true, false] {
        block_on(ruler.frame(input(Some(greet), down)));
    }
    assert!(
        texts(&ruler).iter().any(|(t, _)| t == "Hello, ruler!"),
        "{:?}",
        texts(&ruler)
    );

    // Saved: a new setting, a ruler every five seconds. Reloaded in place;
    // the overlay's key moves, which is what starts it again.
    let dir = ext.installs().get(HELLO).unwrap().dir;
    let manifest = dir.join("extension.toml");
    let mut text = std::fs::read_to_string(&manifest).unwrap();
    text.push_str(
        "\n[[sections.settings]]\nkey = \"example.hello.name\"\ntitle = \"Your name\"\n\
         control = \"text\"\ndefault = \"\"\n",
    );
    std::fs::write(&manifest, text).unwrap();
    let script = dir.join("ruler.luau");
    let source = std::fs::read_to_string(&script).unwrap();
    std::fs::write(
        &script,
        source.replace("local every = 2", "local every = 5"),
    )
    .unwrap();
    let events = ext.reload_changed();
    let [Event::Loaded(change)] = events.as_slice() else {
        panic!("one reload: {events:?}");
    };
    assert!(change.reloaded);
    assert_eq!(change.kept, [GREET]);
    assert_eq!(hello_keys(&sections).len(), 3);
    let [reloaded] = overlays(ext.host(), TIMELINE).try_into().unwrap();
    assert_ne!(reloaded.key(), overlay.key());
    assert_eq!(ticks(&start(&reloaded, &list)), [0.0, 200.0, 400.0]);
    assert!(ext.reload_changed().is_empty(), "nothing changed since");

    // A save that does not parse keeps the last good copy.
    std::fs::write(&manifest, "id = ").unwrap();
    assert!(matches!(
        ext.reload_changed().as_slice(),
        [Event::Failed { id, .. }] if id == HELLO
    ));
    assert_eq!(hello_keys(&sections).len(), 3);
    assert_eq!(run(&list, GREET, &json!({})).unwrap(), "Hello, you!");

    // Removed: its action, its settings and its overlay go with it.
    let removed = run(&list, "EXT_UNINSTALL", &json!({ "id": HELLO })).unwrap();
    assert_eq!(removed["removed"], true);
    assert!(hello_keys(&sections).is_empty());
    assert!(list.info(GREET).is_none());
    assert!(overlays(ext.host(), TIMELINE).is_empty());
}

#[test]
fn from_a_folder_registry_it_loads_where_it_is() {
    let tmp = tempfile::tempdir().unwrap();
    let (list, sections, ext) = app(tmp.path(), FolderRegistry::new(example_registry()));
    ext.install(HELLO, &architect_extensions::VersionReq::STAR)
        .unwrap();
    let install = ext.installs().get(HELLO).unwrap();
    assert!(install.linked, "an edit in the registry is what loads");
    assert_eq!(hello_keys(&sections).len(), 2);
    assert_eq!(run(&list, GREET, &json!({})).unwrap(), "Hello, you!");
    assert!(ext.uninstall(HELLO).unwrap());
    assert!(
        example_registry().join("hello/extension.toml").is_file(),
        "removing a linked install leaves the registry's copy"
    );
}
