//! The extensions page (#181), clicked on Blitz: the lab's metronome (a
//! Luau extension) and clicker (this crate's program, a process
//! extension), each installed from a registry through the consent
//! prompt, updated when the registry has a newer version, turned off and
//! on, and removed — every step a click on the manager's page
//! (`architect_extensions::page::ManagerPage`). And a project's missing
//! extensions: "Install all", one consent prompt for both
//! (`architect_extensions::page::MissingExtensions`).
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_panics_doc
)]

use std::fmt::Write as _;
use std::path::Path;
use std::time::{Duration, Instant};

use architect::action::ActionList;
use architect_extensions::browse::Browse;
use architect_extensions::consent::Grants;
use architect_extensions::extensions::Extensions;
use architect_extensions::host::{Host, SharedSections};
use architect_extensions::install::{FolderRegistry, Installs};
use architect_extensions::manager::{Manager, kinds};
use architect_extensions::page::{ManagerPage, MissingExtensions, use_manager_page};
use architect_extensions::process::ProcessRunner;
use architect_extensions::project::{Plan, Requirement, Requirements, plan};
use blitz_dom::{BaseDocument, Document as _, DocumentConfig};
use blitz_traits::events::{
    BlitzPointerEvent, BlitzPointerId, Modifiers, MouseEventButton, MouseEventButtons, Point,
    PointerCoords, PointerDetails, UiEvent,
};
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native_dom::DioxusDocument;

const METRONOME: &str = "fts.metronome";
const CLICKER: &str = "lab.clicker";

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

/// The metronome's folder at `version`, from the lab's registry.
fn metronome(dir: &Path, version: &str) {
    let lab = Path::new(env!("CARGO_MANIFEST_DIR")).join("../extensions/metronome");
    copy(&lab, dir);
    let manifest = dir.join("extension.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replacen(
            "version = \"0.1.0\"",
            &format!("version = \"{version}\""),
            1,
        ),
    )
    .unwrap();
}

/// The clicker's package at `version`: its manifest and its program.
fn clicker(dir: &Path, version: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("extension.toml"),
        include_str!("../extension.toml").replacen(
            "version = \"0.1.0\"",
            &format!("version = \"{version}\""),
            1,
        ),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        env!("CARGO_BIN_EXE_architect-lab-clicker"),
        dir.join("architect-lab-clicker"),
    )
    .unwrap();
}

/// A registry folder offering both at each of `versions`.
fn registry(root: &Path, versions: &[&str]) {
    let mut index = String::from("name = \"Lab test\"\n");
    for v in versions {
        let (m, c) = (
            root.join(format!("metronome-{v}")),
            root.join(format!("clicker-{v}")),
        );
        if !m.exists() {
            metronome(&m, v);
            clicker(&c, v);
        }
        let _ = write!(
            index,
            "\n[[extensions]]\nid = \"{METRONOME}\"\nname = \"Metronome\"\nversion = \"{v}\"\nkind = \"luau\"\ndescription = \"A click.\"\npath = \"metronome-{v}\"\n\
             \n[[extensions]]\nid = \"{CLICKER}\"\nname = \"Clicker\"\nversion = \"{v}\"\nkind = \"process\"\ndescription = \"A counter.\"\npath = \"clicker-{v}\"\n"
        );
    }
    std::fs::write(root.join("index.toml"), index).unwrap();
}

/// The app: a host that runs processes and keeps grants, its extensions
/// from `reg`, and a manager with the extension kind.
fn app(tmp: &Path, reg: &Path) -> (Extensions, Manager) {
    let list = ActionList::new();
    architect::action::app_status::register(&list).unwrap();
    let runner = ProcessRunner::new().unwrap().with_socket_dir(tmp);
    let host = Host::new(
        list,
        SharedSections::new(architect_settings::Registry::new()),
    )
    .with_runner(runner)
    .with_grants(Grants::open(tmp.join("grants.json")));
    let ext = Extensions::new(host, Installs::new(tmp.join("installs")))
        .with_source(FolderRegistry::new(reg));
    ext.install_actions().unwrap();
    let _ = ext.sync();
    let manager = Manager::new();
    manager.register(kinds::ExtensionKind::new(Browse::new(ext.clone())));
    (ext, manager)
}

#[derive(Props, Clone)]
struct PageProps {
    manager: Manager,
    browse: Browse,
    /// A project's plan: the missing-extensions prompt in place of the
    /// manager's page.
    plan: Option<Plan>,
}

impl PartialEq for PageProps {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

#[allow(non_snake_case, clippy::needless_pass_by_value)]
fn Page(props: PageProps) -> Element {
    use_manager_page(props.manager.clone(), props.browse.clone());
    match props.plan {
        Some(plan) => rsx! { MissingExtensions { browse: props.browse, plan } },
        None => rsx! { ManagerPage { manager: props.manager } },
    }
}

fn mount(props: PageProps) -> DioxusDocument {
    let mut doc = DioxusDocument::new(
        VirtualDom::new_with_props(Page, props),
        DocumentConfig {
            viewport: Some(Viewport::new(1200, 2400, 1.0, ColorScheme::Light)),
            ..Default::default()
        },
    );
    doc.initial_build();
    doc
}

fn settle(doc: &mut DioxusDocument) {
    let waker = futures::task::noop_waker();
    for _ in 0..16 {
        if !doc.poll(Some(std::task::Context::from_waker(&waker))) {
            break;
        }
    }
    doc.inner.borrow_mut().resolve(0.0);
}

fn pump(doc: &mut DioxusDocument, what: &str, until: impl Fn(&BaseDocument) -> bool) {
    let started = Instant::now();
    loop {
        settle(doc);
        if until(&doc.inner.borrow()) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "timed out waiting for {what}; the page says: {}",
            text(&doc.inner.borrow())
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Everything the page says.
fn text(doc: &BaseDocument) -> String {
    doc.root_element().text_content()
}

/// The elements matching `selector` whose text is `label`, in document
/// order.
fn with_text(doc: &BaseDocument, selector: &str, label: &str) -> Vec<blitz_dom::NodeId> {
    doc.query_selector_all(selector)
        .unwrap()
        .into_iter()
        .filter(|id| {
            doc.get_node(*id)
                .is_some_and(|n| n.text_content().trim() == label)
        })
        .collect()
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

/// Clicks the middle of the first `selector` whose text is `label`.
fn click(doc: &mut DioxusDocument, selector: &str, label: &str) {
    settle(doc);
    let (x, y) = {
        let d = doc.inner.borrow();
        let Some(node) = with_text(&d, selector, label).first().copied() else {
            panic!(
                "no {selector} saying {label:?}; the page says: {}",
                text(&d)
            );
        };
        let n = d.get_node(node).unwrap();
        let at = n.absolute_position(0.0, 0.0);
        let size = n.final_layout().size;
        (at.x + size.width / 2.0, at.y + size.height / 2.0)
    };
    doc.handle_ui_event(UiEvent::PointerMove(pointer(x, y, MouseEventButtons::None)));
    doc.handle_ui_event(UiEvent::PointerDown(pointer(
        x,
        y,
        MouseEventButtons::Primary,
    )));
    doc.handle_ui_event(UiEvent::PointerUp(pointer(x, y, MouseEventButtons::None)));
    settle(doc);
}

/// The consent prompt is up, asking to `action` (`Install`, `Update`).
fn asked(d: &BaseDocument, action: &str, name: &str) -> bool {
    text(d).contains(&format!("{action} {name} ")) && text(d).contains("It will be able to:")
}

fn installed(ext: &Extensions, id: &str) -> Option<String> {
    ext.installs().get(id).map(|i| i.version.to_string())
}

/// One extension, through its page: install (asked first), update,
/// off, on, remove.
fn through_the_page(doc: &mut DioxusDocument, ext: &Extensions, reg: &Path, id: &str, name: &str) {
    // Picked from the list: offered by the registry.
    click(doc, "p", name);
    pump(doc, "its page", |d| {
        !with_text(d, "button", "Install").is_empty()
    });
    assert_eq!(installed(ext, id), None);

    // Install: the consent prompt first (above the page, so its button is
    // the first "Install"), then installed and running.
    click(doc, "button", "Install");
    pump(doc, "the consent prompt", |d| asked(d, "Install", name));
    assert_eq!(installed(ext, id), None, "not before a yes");
    click(doc, "button", "Install");
    pump(doc, "installed", |d| {
        text(d).contains(&format!("{name} 0.1.0 installed and running"))
    });
    assert_eq!(installed(ext, id).as_deref(), Some("0.1.0"));
    assert!(ext.host().get(id).is_some(), "{id} runs");
    assert!(ext.host().grants().unwrap().granted(id).is_some());

    // The registry has a newer version: Update takes it.
    registry(reg, &["0.1.0", "0.2.0"]);
    click(doc, "button", "Update");
    pump(doc, "the update", |d| {
        text(d).contains("Updated to 0.2.0") || asked(d, "Update", name)
    });
    if asked(&doc.inner.borrow(), "Update", name) {
        click(doc, "button", "Update");
    }
    pump(doc, "0.2.0", |_| {
        installed(ext, id).as_deref() == Some("0.2.0")
    });
    assert_eq!(
        ext.host().get(id).unwrap().manifest.version.to_string(),
        "0.2.0"
    );

    // Off: unloaded, still installed. On: loaded again.
    click(doc, "button", "Turn off");
    pump(doc, "turned off", |d| {
        !with_text(d, "button", "Turn on").is_empty()
    });
    assert!(ext.host().get(id).is_none(), "{id} off");
    assert_eq!(installed(ext, id).as_deref(), Some("0.2.0"));
    click(doc, "button", "Turn on");
    pump(doc, "turned on", |d| {
        !with_text(d, "button", "Turn off").is_empty()
    });
    assert!(ext.host().get(id).is_some(), "{id} on");

    // Remove: gone, offered again.
    click(doc, "button", "Remove");
    pump(doc, "removed", |d| {
        text(d).contains("Removed.") && !with_text(d, "button", "Install").is_empty()
    });
    assert_eq!(installed(ext, id), None);
    assert!(ext.host().get(id).is_none(), "{id} gone");
    registry(reg, &["0.1.0"]);
}

#[test]
fn the_metronome_and_the_clicker_install_update_turn_off_and_remove_from_the_page() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = tmp.path().join("registry");
    registry(&reg, &["0.1.0"]);
    let (ext, manager) = app(tmp.path(), &reg);
    let mut doc = mount(PageProps {
        manager,
        browse: Browse::new(ext.clone()),
        plan: None,
    });
    pump(&mut doc, "the list", |d| {
        !with_text(d, "p", "Metronome").is_empty() && !with_text(d, "p", "Clicker").is_empty()
    });
    through_the_page(&mut doc, &ext, &reg, METRONOME, "Metronome");
    through_the_page(&mut doc, &ext, &reg, CLICKER, "Clicker");
}

#[test]
fn a_projects_missing_extensions_install_all_through_one_consent_prompt() {
    let tmp = tempfile::tempdir().unwrap();
    let reg = tmp.path().join("registry");
    registry(&reg, &["0.1.0"]);
    let (ext, manager) = app(tmp.path(), &reg);
    let mut needs = Requirements::default();
    needs.need(Requirement::new(METRONOME).named("Metronome"));
    needs.need(Requirement::new(CLICKER).named("Clicker"));
    let plan = plan(&needs, &[], &ext.offers(), |_| true, None);
    assert_eq!(plan.install.len(), 2, "{plan:?}");
    let mut doc = mount(PageProps {
        manager,
        browse: Browse::new(ext.clone()),
        plan: Some(plan),
    });
    pump(&mut doc, "the prompt", |d| {
        text(d).contains("This project uses Metronome and Clicker")
    });

    click(&mut doc, "button", "Install all");
    pump(&mut doc, "one consent prompt for both", |d| {
        asked(d, "Install", "Metronome 0.1.0, Clicker")
    });
    assert_eq!(installed(&ext, METRONOME), None, "not before a yes");
    click(&mut doc, "button", "Install");
    pump(&mut doc, "both installed", |d| {
        text(d)
            .contains("Metronome 0.1.0 installed and running; Clicker 0.1.0 installed and running")
    });
    for id in [METRONOME, CLICKER] {
        assert_eq!(installed(&ext, id).as_deref(), Some("0.1.0"));
        assert!(ext.host().get(id).is_some(), "{id} runs");
        assert!(
            ext.host().grants().unwrap().granted(id).is_some(),
            "{id} granted"
        );
    }
}
