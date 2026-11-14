//! The issue's bar (#100): an extension in a process of its own shows a
//! view in a pane that reacts to clicks, and killing its process leaves
//! the app running, the pane saying why.
//!
//! The extension is this test binary, started again as a child: the
//! `guest` test serves when the host's socket is in its environment and
//! does nothing otherwise. The host is a headless Dioxus document on
//! Blitz, mounting [`RemoteView`] as an app's pane does; clicks go in
//! through the host document, which forwards them to the sub-document.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_panics_doc,
    clippy::future_not_send,
    clippy::arithmetic_side_effects,
    clippy::significant_drop_tightening
)]

use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::guest::Guest;
use architect_extensions::host::{Host, LoadError, SharedSections};
use architect_extensions::process::{ProcessRunner, Processes, Restart, State};
use architect_extensions::proto::SOCKET;
use architect_extensions::remote::{RemoteDocument, RemoteEmbed, RemoteView, mounted_views};
use architect_extensions::wire::{Edit, Frame, Value};
use architect_extensions::{Origin, Package};
use blitz_dom::{BaseDocument, Document, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;
use futures::executor::block_on;
use serde_json::json;

const ID: &str = "test.clicker";
const CLICKS: &str = "TEST_CLICKER_CLICKS";

static CLICKED: AtomicU32 = AtomicU32::new(0);

/// The extension's view: a button, what it counted, and parts that come
/// and go with the count (a placeholder replaced, a list grown).
#[component]
fn Counter() -> Element {
    let mut n = use_signal(|| 0_u32);
    rsx! {
        div {
            button {
                id: "more",
                style: "display: block; width: 120px; height: 40px;",
                onclick: move |_| {
                    n += 1;
                    CLICKED.fetch_add(1, Ordering::Relaxed);
                },
                "More"
            }
            p { id: "count", "Clicked {n} times" }
            if n() % 2 == 1 {
                span { id: "odd", "odd" }
            }
            ul {
                for i in 0..n() {
                    li { key: "{i}", "row {i}" }
                }
            }
        }
    }
}

/// A view drawn as an inline embed: a row 20 points tall for each line
/// of the block it was given.
#[component]
fn Lines() -> Element {
    let input = architect_extensions::linked::use_embed();
    let source = input.map(|i| i.source).unwrap_or_default();
    rsx! {
        div { id: "lines",
            for (i, line) in source.lines().enumerate() {
                div { key: "{i}", class: "line", style: "height: 20px;", "{line}" }
            }
        }
    }
}

/// Not a test of its own: the extension, when a test starts this binary
/// as one.
#[test]
fn guest() {
    if std::env::var_os(SOCKET).is_none() {
        return;
    }
    Guest::new()
        .view("counter", Counter)
        .view("lines", Lines)
        .action(CLICKS, |_| async {
            Ok(json!(CLICKED.load(Ordering::Relaxed)))
        })
        .serve()
        .unwrap();
}

/// A package of this binary as a process extension, its manifest
/// declaring `views`.
fn package(dir: &Path, views: &[&str]) -> Package {
    let exe = std::env::current_exe().unwrap();
    std::os::unix::fs::symlink(exe, dir.join("guest")).unwrap();
    let mut manifest = format!(
        r#"id = "{ID}"
name = "Clicker"
version = "1.0.0"
kind = "process"

[entry]
path = "guest"
args = ["guest", "--exact", "--nocapture", "--test-threads=1"]

[[actions]]
id = "{CLICKS}"
name = "Clicker: clicks"
effect = "read_only"
"#
    );
    for v in views {
        let _ = write!(manifest, "\n[[views]]\nid = \"{v}\"\ntitle = \"{v}\"\n");
    }
    std::fs::write(dir.join("extension.toml"), manifest).unwrap();
    Package::read(dir).unwrap()
}

/// A host running process extensions, never restarting one on its own.
fn host(sockets: &Path) -> (Host, Processes) {
    let runner = ProcessRunner::new()
        .unwrap()
        .with_socket_dir(sockets)
        .with_restart(Restart::never());
    let processes = runner.processes();
    let host = Host::new(ActionList::new(), SharedSections::default()).with_runner(runner);
    (host, processes)
}

#[derive(Props, Clone, PartialEq)]
struct PaneProps {
    processes: Processes,
}

/// The app: one pane, the view in it.
#[allow(non_snake_case)]
fn Pane(props: PaneProps) -> Element {
    rsx! {
        div { style: "width: 400px; height: 300px;",
            RemoteView { processes: props.processes, extension: ID, view: "counter" }
        }
    }
}

fn app(processes: Processes) -> DioxusDocument {
    let vdom = VirtualDom::new_with_props(Pane, PaneProps { processes });
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc
}

/// Runs the app (its tasks, its layout) until `until` holds.
fn pump(doc: &mut DioxusDocument, what: &str, until: impl Fn(&BaseDocument) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let waker = futures::task::noop_waker();
    loop {
        for _ in 0..16 {
            if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
                break;
            }
        }
        doc.inner.borrow_mut().resolve(0.0);
        if until(&doc.inner.borrow()) {
            return;
        }
        if Instant::now() >= deadline {
            let d = doc.inner.borrow();
            panic!(
                "timed out waiting for {what}: the pane says {:?}, {} views mounted, the view's count {:?}",
                in_app(&d, ".remote-view-why"),
                mounted_views(&d).len(),
                in_view(&d, "#count"),
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The text of `selector` in the pane's view.
fn in_view(doc: &BaseDocument, selector: &str) -> Option<String> {
    let at = *mounted_views(doc).first()?;
    let view = doc.subdoc(at)?.inner();
    let node = view.query_selector(selector).ok()??;
    Some(view.get_node(node)?.text_content())
}

/// The text of `selector` in the app's own document.
fn in_app(doc: &BaseDocument, selector: &str) -> Option<String> {
    let node = doc.query_selector(selector).ok()??;
    Some(doc.get_node(node)?.text_content())
}

/// The middle of `node` in `doc`.
fn middle(doc: &BaseDocument, node: blitz_dom::NodeId) -> (f32, f32) {
    let n = doc.get_node(node).unwrap();
    let at = n.absolute_position(0.0, 0.0);
    let size = n.final_layout().size;
    (at.x + size.width / 2.0, at.y + size.height / 2.0)
}

/// Where the view's `selector` is, in the app's window.
fn view_point(doc: &BaseDocument, selector: &str) -> (f32, f32) {
    let at = mounted_views(doc)[0];
    let host = doc.get_node(at).unwrap().absolute_position(0.0, 0.0);
    let view = doc.subdoc(at).unwrap().inner();
    let node = view.query_selector(selector).unwrap().unwrap();
    let (x, y) = middle(&view, node);
    (host.x + x, host.y + y)
}

fn pointer(x: f32, y: f32, buttons: MouseEventButtons) -> BlitzPointerEvent {
    BlitzPointerEvent {
        id: BlitzPointerId::Mouse,
        is_primary: true,
        coords: PointerCoords {
            page_x: x,
            page_y: y,
            screen_x: x,
            screen_y: y,
            client_x: x,
            client_y: y,
        },
        button: MouseEventButton::Main,
        buttons,
        mods: Modifiers::empty(),
        details: PointerDetails::default(),
        element: Point { x: 0.0, y: 0.0 },
        active_pointers: std::sync::Arc::default(),
    }
}

/// A click at `(x, y)` in the app's window.
fn click(doc: &mut DioxusDocument, (x, y): (f32, f32)) {
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, MouseEventButtons::None)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(
        x,
        y,
        MouseEventButtons::Primary,
    )));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(x, y, MouseEventButtons::None)));
}

fn clicks(host: &Host) -> Result<String, String> {
    block_on(host.list().run_json_async(CLICKS, b""))
        .map(|out| String::from_utf8(out).unwrap())
        .map_err(|e| e.to_string())
}

#[test]
fn a_process_view_reacts_to_clicks_and_its_death_leaves_the_app_running() {
    let dir = tempfile::tempdir().unwrap();
    let (host, processes) = host(dir.path());
    let loaded = host
        .load(&package(dir.path(), &["counter"]), Origin::Installed)
        .unwrap();
    assert_eq!(loaded.added, [CLICKS]);
    let process = processes.get(ID).unwrap();
    assert!(matches!(process.state(), State::Running { .. }));

    // Its view, in a pane, from its process.
    let mut doc = app(processes.clone());
    pump(&mut doc, "the view", |d| {
        in_view(d, "#count").as_deref() == Some("Clicked 0 times")
    });
    assert_eq!(in_app(&doc.inner.borrow(), ".remote-view-why"), None);

    // Clicks reach its handlers, and what they change comes back.
    let more = view_point(&doc.inner.borrow(), "#more");
    click(&mut doc, more);
    pump(&mut doc, "the first click", |d| {
        in_view(d, "#count").as_deref() == Some("Clicked 1 times")
            && in_view(d, "#odd").as_deref() == Some("odd")
    });
    click(&mut doc, more);
    pump(&mut doc, "the second click", |d| {
        in_view(d, "#count").as_deref() == Some("Clicked 2 times")
            && in_view(d, "#odd").is_none()
            && in_view(d, "li:nth-child(2)").as_deref() == Some("row 1")
    });
    // Its action ran in the same process, over vox.
    assert_eq!(clicks(&host).unwrap(), "2");

    // Killed, it leaves the app running and the pane saying why.
    let pid = process.pid().unwrap();
    let killed = std::process::Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(killed.success());
    pump(&mut doc, "the placeholder", |d| {
        mounted_views(d).is_empty()
            && in_app(d, ".remote-view-why")
                .is_some_and(|why| why.contains("Clicker stopped: killed by signal 9"))
    });
    assert!(matches!(
        process.state(),
        State::Exited {
            restarting: false,
            ..
        }
    ));
    let gone = clicks(&host).unwrap_err();
    assert!(gone.contains("not running"), "{gone}");
    // The host itself is untouched: the extension is still loaded.
    assert_eq!(host.ids(), [ID]);

    // The placeholder's button starts it again, and the view comes back.
    let again = {
        let d = doc.inner.borrow();
        let button = d.query_selector(".remote-view-restart").unwrap().unwrap();
        middle(&d, button)
    };
    click(&mut doc, again);
    pump(&mut doc, "the view again", |d| {
        in_view(d, "#count").as_deref() == Some("Clicked 0 times")
    });
    assert_eq!(clicks(&host).unwrap(), "0");

    // Unloaded, its process stops and the pane says so.
    assert!(host.unload(ID));
    pump(&mut doc, "the unloaded placeholder", |d| {
        mounted_views(d).is_empty() && in_app(d, ".remote-view-why").is_some()
    });
    assert!(processes.get(ID).is_none());
}

#[test]
fn a_process_that_lacks_a_declared_view_does_not_load() {
    let dir = tempfile::tempdir().unwrap();
    let (host, processes) = host(dir.path());
    let error = host
        .load(
            &package(dir.path(), &["counter", "missing"]),
            Origin::Installed,
        )
        .unwrap_err();
    assert_eq!(
        error,
        LoadError::Start("it does not draw its view missing".to_owned())
    );
    assert!(host.ids().is_empty());
    assert!(processes.ids().is_empty());
}

#[test]
fn a_program_that_never_says_hello_does_not_load() {
    let dir = tempfile::tempdir().unwrap();
    let (host, _) = host(dir.path());
    let mut package = package(dir.path(), &[]);
    // This binary again, asked for a test it does not have: the harness
    // runs nothing and exits at once, saying nothing.
    package.manifest.entry.args = vec!["--exact".to_owned(), "no_such_test".to_owned()];
    let error = host.load(&package, Origin::Installed).unwrap_err();
    assert_eq!(error, LoadError::Start("exited with code 0".to_owned()));
}

/// A frame that builds `<div id="a">x</div>` under the root, as element 1.
fn div() -> Frame {
    Frame::Edits {
        edits: vec![
            Edit::PushId { id: 0 },
            Edit::CreateElement {
                tag: "div".to_owned(),
                namespace: None,
            },
            Edit::SetId { id: 1 },
            Edit::CreateText {
                value: "x".to_owned(),
            },
            Edit::AppendChildren { m: 1 },
            Edit::SetAttribute {
                name: "id".to_owned(),
                namespace: None,
                value: Value::Text("a".to_owned()),
            },
            Edit::AppendChildren { m: 1 },
            Edit::Pop,
        ],
    }
}

#[test]
fn a_view_that_sends_nonsense_fails_and_never_takes_the_app_down() {
    let fresh = || {
        let (events, _) = futures::channel::mpsc::unbounded();
        let mut view = RemoteDocument::new(events);
        view.apply(div()).unwrap();
        view
    };
    let view = fresh();
    let at = view.document().query_selector("#a").unwrap().unwrap();
    assert_eq!(view.document().get_node(at).unwrap().text_content(), "x");

    let edits = |edits: Vec<Edit>| Frame::Edits { edits };
    let nonsense: Vec<(&str, Frame)> = vec![
        (
            "an element it never made",
            edits(vec![Edit::PushId { id: 99 }]),
        ),
        ("the root", edits(vec![Edit::Remove])),
        (
            "the root, pushed",
            edits(vec![Edit::PushId { id: 0 }, Edit::Remove]),
        ),
        (
            "more nodes than the stack",
            edits(vec![Edit::AppendChildren { m: 5 }]),
        ),
        (
            "a node into itself",
            edits(vec![
                Edit::PushId { id: 1 },
                Edit::PushId { id: 1 },
                Edit::AppendChildren { m: 1 },
            ]),
        ),
        (
            "the root into the view",
            edits(vec![
                Edit::PushId { id: 1 },
                Edit::PushId { id: 0 },
                Edit::AppendChildren { m: 1 },
            ]),
        ),
        (
            "a child that is not there",
            edits(vec![Edit::PushId { id: 1 }, Edit::Child { index: 7 }]),
        ),
        ("a pop past the root", edits(vec![Edit::Pop])),
        (
            "a handler on a node with no id",
            edits(vec![
                Edit::CreateElement {
                    tag: "div".to_owned(),
                    namespace: None,
                },
                Edit::Listen {
                    name: "click".to_owned(),
                },
            ]),
        ),
        (
            "raw HTML",
            edits(vec![
                Edit::PushId { id: 1 },
                Edit::SetAttribute {
                    name: "dangerous_inner_html".to_owned(),
                    namespace: None,
                    value: Value::Text("<script></script>".to_owned()),
                },
            ]),
        ),
        (
            "a failure of its own",
            Frame::Failed("its component panicked".to_owned()),
        ),
    ];
    for (what, frame) in nonsense {
        assert!(fresh().apply(frame).is_err(), "{what} was applied");
    }
}

/// The block an embed is given, as the host's note has it.
static LINES: GlobalSignal<String> = Signal::global(|| "a\nb\nc".to_owned());

/// The app: a note with the process's view inline, as an embed.
#[allow(non_snake_case)]
fn Note(props: PaneProps) -> Element {
    let mut input = use_signal(architect_extensions::EmbedInput::default);
    use_effect(move || {
        input.set(architect_extensions::EmbedInput {
            claim: "fence:lines".to_owned(),
            source: LINES(),
            props: serde_json::Value::Null,
        });
    });
    rsx! {
        div { style: "width: 400px; display: flex; flex-direction: column;",
            div { id: "embed",
                RemoteEmbed { processes: props.processes, extension: ID, view: "lines", input, max_height: 120 }
            }
        }
    }
}

fn embed_height(doc: &BaseDocument) -> f32 {
    let node = doc.query_selector("#embed").unwrap().unwrap();
    doc.get_node(node).unwrap().final_layout().size.height
}

fn lines(doc: &BaseDocument) -> usize {
    let Some(at) = mounted_views(doc).first().copied() else {
        return 0;
    };
    let view = doc.subdoc(at).unwrap().inner();
    view.query_selector_all(".line").map_or(0, |l| l.len())
}

#[test]
fn a_process_view_inline_as_an_embed_is_given_its_block_and_sized_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let (host, processes) = host(dir.path());
    host.load(&package(dir.path(), &["lines"]), Origin::Installed)
        .unwrap();
    let vdom = VirtualDom::new_with_props(Note, PaneProps { processes });
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    pump(&mut doc, "three lines, 60 points tall", |d| {
        lines(d) == 3 && (embed_height(d) - 60.0).abs() < 0.5
    });
    doc.vdom
        .in_runtime(|| *LINES.write() = "a\nb\nc\nd\ne".to_owned());
    pump(&mut doc, "five lines, 100 points tall", |d| {
        lines(d) == 5 && (embed_height(d) - 100.0).abs() < 0.5
    });
    doc.vdom.in_runtime(|| *LINES.write() = "x\n".repeat(12));
    pump(&mut doc, "twelve lines, at most 120 points tall", |d| {
        lines(d) == 12 && (embed_height(d) - 120.0).abs() < 0.5
    });
}

/// The view a [`Switching`] pane shows: the next tab of its group.
static SHOWN: GlobalSignal<&'static str> = Signal::global(|| "counter");

/// The app: one pane whose tab goes from one process view to another,
/// in the same place, as a group's current tab does.
#[allow(non_snake_case)]
fn Switching(props: PaneProps) -> Element {
    rsx! {
        div { style: "width: 400px; height: 300px;",
            RemoteView { processes: props.processes, extension: ID, view: SHOWN() }
        }
    }
}

#[test]
fn a_pane_that_goes_on_to_another_process_view_shows_that_view() {
    let dir = tempfile::tempdir().unwrap();
    let (host, processes) = host(dir.path());
    host.load(
        &package(dir.path(), &["counter", "lines"]),
        Origin::Installed,
    )
    .unwrap();
    let vdom = VirtualDom::new_with_props(Switching, PaneProps { processes });
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(800, 600, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    pump(&mut doc, "the counter", |d| {
        in_view(d, "#count").as_deref() == Some("Clicked 0 times")
    });
    doc.vdom.in_runtime(|| *SHOWN.write() = "lines");
    pump(&mut doc, "the lines view in its place", |d| {
        in_view(d, "#lines").is_some() && in_view(d, "#count").is_none()
    });
}

/// `node`'s computed `property` in `view`'s document, laid out.
fn computed(view: &mut RemoteDocument, selector: &str, property: &str) -> String {
    view.inner_mut().resolve(0.0);
    let doc = view.inner();
    let node = doc.query_selector(selector).unwrap().unwrap();
    doc.resolved_style_value(node, property)
}

#[test]
fn a_view_that_sets_no_colour_draws_in_the_panes_ink_and_font() {
    let (events, _) = futures::channel::mpsc::unbounded();
    let mut view = RemoteDocument::new(events);
    view.set_theme_css(":root { --ink: rgb(1, 2, 3); --font: monospace; }");
    view.apply(div()).unwrap();
    assert_eq!(computed(&mut view, "#a", "color"), "rgb(1, 2, 3)");
    assert_eq!(computed(&mut view, "#a", "font-size"), "13px");
    assert_eq!(computed(&mut view, "#a", "font-family"), "monospace");
    assert_eq!(computed(&mut view, "body", "margin-top"), "0px");
}

#[test]
fn a_styles_raw_css_is_its_sheet_and_other_raw_html_still_fails() {
    let (events, _) = futures::channel::mpsc::unbounded();
    let mut view = RemoteDocument::new(events);
    view.apply(div()).unwrap();
    // `style { dangerous_inner_html: CSS }`, as an editor ships its sheet.
    let sheet = |css: &str| Frame::Edits {
        edits: vec![
            Edit::PushId { id: 0 },
            Edit::CreateElement {
                tag: "style".to_owned(),
                namespace: None,
            },
            Edit::SetAttribute {
                name: "dangerous_inner_html".to_owned(),
                namespace: None,
                value: Value::Text(css.to_owned()),
            },
            Edit::AppendChildren { m: 1 },
            Edit::Pop,
        ],
    };
    view.apply(sheet("#a { color: rgb(4, 5, 6); }")).unwrap();
    assert_eq!(computed(&mut view, "#a", "color"), "rgb(4, 5, 6)");
    // Markup in it stays text: no element comes of it.
    view.apply(sheet("</style><div id=\"b\"></div>")).unwrap();
    assert!(view.inner().query_selector("#b").unwrap().is_none());
    // Anywhere else, raw HTML still fails the view.
    let markup = Frame::Edits {
        edits: vec![
            Edit::PushId { id: 1 },
            Edit::SetAttribute {
                name: "dangerous_inner_html".to_owned(),
                namespace: None,
                value: Value::Text("<span></span>".to_owned()),
            },
        ],
    };
    assert!(view.apply(markup).is_err());
}
