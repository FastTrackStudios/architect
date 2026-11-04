//! On a desk the workspace's top tab bars are the window's title row
//! (Obsidian's): laid out by Blitz under the app's shell, the two panes'
//! tab bars sit at the window's top, as tall as the shell's row, and there
//! is no second row under them; the first clears the sidebar's button. A
//! page with one tab keeps it in the shell's own row.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use architect_shell::{
    Account, AppShell, Insets, Menu, Page, Screen, SidebarDefault, Tab, use_shell,
    use_sidebar_default,
};
use architect_workspace::{
    Dest, Pane, PaneId, Popout, Side, Target, TitleRow, View, Views, WindowId, WindowSystem,
    Windows, Workspace, WorkspaceView,
};
use blitz_dom::Document as _;
use blitz_traits::shell::{ColorScheme, Viewport};
use dioxus::prelude::*;
use dioxus_native::{DioxusDocument, DocumentConfig};
use lucide_dioxus::Inbox;

const DESK: Screen = Screen {
    width: 1280.0,
    height: 800.0,
    touch: false,
    insets: Insets {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    },
};

#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

#[component]
fn Board() -> Element {
    use_sidebar_default(SidebarDefault::Collapsed);
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        let _ = w.open(Pane::new("editor", "note", "Editor"), Target::Tab);
        let _ = w.open(
            Pane::new("chart", "note", "Chart"),
            Target::Split(Side::Right),
        );
        w
    });
    let views = Views::new().view(View::new("note", "Note", |_| rsx! { div { "a note" } }).many());
    let shell = use_shell();
    let (left, right) = shell.title_clearance();
    let title_row = TitleRow {
        height: narrow(shell.title_row()),
        left: narrow(left),
        right: narrow(right),
        sidebar_left: narrow(shell.sidebar_title_left()),
        on_drag: None,
        on_zoom: None,
    };
    rsx! {
        div { style: "flex: 1 1 0; min-height: 0; display: flex; position: relative;",
            WorkspaceView { workspace, views, title_row, in_shell: true }
        }
    }
}

#[component]
fn InboxPage() -> Element {
    rsx! { Page { title: "Inbox", div { "the inbox" } } }
}

fn shell(page: Element) -> Element {
    {
        rsx! {
            div { style: "position: absolute; inset: 0;",
                AppShell {
                    screen: DESK,
                    tabs: vec![Tab::new("vault", "Vault", architect_shell::icon!(Inbox))],
                    current: Some("vault"),
                    on_tab: |_| (),
                    account: Account::new("Cody Wright").menu(Menu::new()),
                    left: rsx! { div { "the left side" } },
                    {page}
                }
            }
        }
    }
}

fn board_app() -> Element {
    shell(rsx! { Board {} })
}

fn inbox_app() -> Element {
    shell(rsx! { InboxPage {} })
}

fn laid_out(app: fn() -> Element) -> DioxusDocument {
    // The sidebar folds at once, not on a spring no frame clock drives here.
    architect_shell::motion::set_reduced_motion(Some(true));
    let vdom = VirtualDom::new(app);
    let mut doc = DioxusDocument::new(
        vdom,
        DocumentConfig {
            viewport: Some(Viewport::new(1280, 800, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    // The page's claims land on the shell, which draws again.
    for _ in 0..4 {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
    }
    doc
}

/// `(x, y, width, height)` of every element matching `selector`.
fn rects(doc: &DioxusDocument, selector: &str) -> Vec<(f64, f64, f64, f64)> {
    let d = doc.inner.borrow();
    d.query_selector_all(selector)
        .unwrap()
        .into_iter()
        .filter_map(|id| d.get_client_bounding_rect(id))
        .map(|r| (r.x, r.y, r.width, r.height))
        .collect()
}

/// Whether the element on top at `(x, y)` is in the one `selector` names.
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
fn hits_inside(doc: &DioxusDocument, x: f64, y: f64, selector: &str) -> bool {
    let d = doc.inner.borrow();
    let Some(target) = d.query_selector(selector).unwrap() else {
        return false;
    };
    d.hit(x as f32, y as f32)
        .is_some_and(|hit| d.node_chain(hit.node_id).contains(&target))
}

#[test]
fn the_panes_tab_bars_are_the_title_row_one_row() {
    let doc = laid_out(board_app);
    // No window of its own here: the shell's row is 52pt.
    let row = 52.0;
    let bars = rects(&doc, "[data-tab-bar]");
    assert_eq!(bars.len(), 2, "two panes, two tab bars: {bars:?}");
    for (x, y, _, h) in &bars {
        assert!(
            y.abs() < 0.5,
            "a tab bar at the window's top, not under a row: {bars:?}"
        );
        assert!((h - row).abs() < 0.5, "as tall as the title row: {bars:?}");
        assert!(*x >= 44.0, "past the ribbon: {bars:?}");
    }
    // The first tab clears the window's corner and the sidebar's button.
    let first = rects(&doc, "[data-tab=\"editor\"]");
    let toggle = rects(&doc, "[aria-label=\"Toggle the sidebar\"]");
    assert_eq!(toggle.len(), 1);
    // (Blitz pads a button past its 28pt; the icon is in the 28.)
    let (tx, _, _, _) = toggle[0];
    assert!(
        first[0].0 >= tx + 28.0 + 8.0,
        "the first tab {first:?} clears the toggle {toggle:?}"
    );
    // And drawn on top: a press there lands on the tab, not the shell's row.
    let (x, y, w, h) = first[0];
    assert!(
        hits_inside(&doc, x + w / 2.0, y + h / 2.0, "[data-tab=\"editor\"]"),
        "the tab is on top"
    );
    // The panes (each its own header, then its view) start under the one row.
    let notes = rects(&doc, "[data-pane]");
    assert!(!notes.is_empty());
    for (_, y, _, _) in notes {
        assert!(y >= row - 0.5, "a pane under the row, at {y}");
    }
    // Folded on this page: the account is a mark on the ribbon.
    assert_eq!(
        rects(&doc, "[aria-label=\"Account: Cody Wright\"]").len(),
        1
    );
    assert!(rects(&doc, "[aria-label=\"Account\"]").is_empty());
}

#[test]
fn a_page_with_one_tab_keeps_it_in_the_shells_row() {
    let doc = laid_out(inbox_app);
    let tab = rects(&doc, "[data-desk-tab=\"Inbox\"]");
    assert_eq!(tab.len(), 1, "the page's tab: {tab:?}");
    let (_, y, _, h) = tab[0];
    assert!(y >= 0.0 && y + h <= 52.5, "in the title row: {tab:?}");
    assert!(rects(&doc, "[data-tab-bar]").is_empty());
    // The app's sidebar is open beside it, its account a row.
    assert_eq!(rects(&doc, "[aria-label=\"Account\"]").len(), 1);
}

// A pop-out window draws its own title bar too (a Mac's): its launcher
// gives it a `TitleRow`, and its tab bar is the row, one row.

/// A platform that keeps what it was asked to open.
#[derive(Default)]
struct Fake(std::cell::RefCell<Vec<Popout>>);

struct Platform(std::rc::Rc<Fake>);

impl WindowSystem for Platform {
    fn open(&self, popout: Popout) {
        self.0.0.borrow_mut().push(popout);
    }
    fn close(&self, _: &WindowId) {}
    fn origin(&self, _: &WindowId) -> Option<(f32, f32)> {
        Some((0.0, 0.0))
    }
    fn window_at(&self, _: f32, _: f32) -> Option<WindowId> {
        None
    }
    fn on_release_outside(&self, _: &WindowId, _: std::rc::Rc<dyn Fn(f32, f32)>) {}
}

thread_local! {
    static FAKE: std::cell::RefCell<Option<std::rc::Rc<Fake>>> = const { std::cell::RefCell::new(None) };
}

fn main_with_popout() -> Element {
    let fake = FAKE.with(|f| f.borrow().clone().unwrap());
    use_context_provider(|| Windows::new(Platform(fake)));
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        let _ = w.open(Pane::new("editor", "note", "Editor"), Target::Tab);
        let _ = w.open(Pane::new("chart", "note", "Chart"), Target::Tab);
        w.move_pane(&PaneId::new("chart"), Dest::Window(None))
            .unwrap();
        w
    });
    let views = use_hook(|| {
        Views::new().view(View::new("note", "Note", |_| rsx! { div { "a note" } }).many())
    });
    rsx! { WorkspaceView { workspace, views } }
}

#[test]
fn a_popout_window_has_its_tabs_in_its_title_row() {
    FAKE.with(|f| *f.borrow_mut() = Some(std::rc::Rc::new(Fake::default())));
    let mut main = VirtualDom::new(main_with_popout);
    main.rebuild_in_place();
    let popout = FAKE.with(|f| f.borrow().as_ref().unwrap().0.borrow()[0].clone());
    let row = TitleRow {
        height: 40.0,
        left: 86.0,
        right: 0.0,
        sidebar_left: 0.0,
        on_drag: None,
        on_zoom: None,
    };
    let mut doc = DioxusDocument::new(
        popout.dom().with_root_context(row),
        DocumentConfig {
            viewport: Some(Viewport::new(640, 480, 1.0, ColorScheme::Dark)),
            ..Default::default()
        },
    );
    doc.initial_build();
    for _ in 0..3 {
        doc.poll(None);
        doc.inner.borrow_mut().resolve(0.0);
    }
    let bars = rects(&doc, "[data-tab-bar]");
    assert_eq!(bars.len(), 1, "{bars:?}");
    // From the window's root (a test document keeps the body's margin).
    let (ox, oy, _, _) = rects(&doc, "[style*=\"100vw\"]")[0];
    let (_, y, _, h) = bars[0];
    assert!(
        (y - oy).abs() < 0.5 && (h - 40.0).abs() < 0.5,
        "the bar is the 40pt row: {bars:?}"
    );
    let tab = rects(&doc, "[data-tab=\"chart\"]");
    assert!(
        tab[0].0 - ox >= 86.0,
        "clear of the traffic lights: {tab:?}"
    );
    for (_, y, _, _) in rects(&doc, "[data-pane]") {
        assert!(y - oy >= 39.5, "the pane under the one row, at {y}");
    }
    drop(doc);
    drop(main);
}
