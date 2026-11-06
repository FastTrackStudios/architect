//! Pop-out windows share the app's state: a pop-out is a second
//! `VirtualDom` (Blitz draws one document per dom) reading the main
//! window's `Signal<Workspace>`. These prove a signal owned by one dom
//! redraws a component in another when it changes, and that a write from
//! the other comes back.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects,
    clippy::needless_pass_by_value
)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use dioxus::core::NoOpMutations;
use dioxus::prelude::*;

thread_local! {
    static SHARED: RefCell<Option<Signal<u32>>> = const { RefCell::new(None) };
    static SEEN: Cell<u32> = const { Cell::new(0) };
    static RENDERS: Cell<u32> = const { Cell::new(0) };
}

fn main_window() -> Element {
    let value = use_signal(|| 1_u32);
    SHARED.with(|s| *s.borrow_mut() = Some(value));
    rsx! { "{value}" }
}

#[derive(Props, Clone, PartialEq)]
struct PopoutProps {
    value: Signal<u32>,
}

#[allow(non_snake_case)]
fn Popout(props: PopoutProps) -> Element {
    RENDERS.with(|r| r.set(r.get() + 1));
    SEEN.with(|s| s.set((props.value)()));
    rsx! { "{props.value}" }
}

#[test]
fn a_signal_from_one_dom_redraws_another() {
    let mut main = VirtualDom::new(main_window);
    main.rebuild_in_place();
    let value = SHARED.with(|s| s.borrow().expect("the main window made it"));

    let mut popout = VirtualDom::new_with_props(Popout, PopoutProps { value });
    popout.rebuild_in_place();
    assert_eq!(SEEN.with(Cell::get), 1);
    let before = RENDERS.with(Cell::get);

    // Written in the main window's runtime: the pop-out redraws.
    main.in_runtime(|| {
        let mut v = value;
        v.set(7);
    });
    popout.render_immediate(&mut NoOpMutations);
    assert_eq!(SEEN.with(Cell::get), 7);
    assert!(RENDERS.with(Cell::get) > before);

    // Written from the pop-out: the main window sees it.
    let wrote = Rc::new(Cell::new(false));
    let w = wrote.clone();
    popout.in_runtime(move || {
        let mut v = value;
        v.set(9);
        w.set(true);
    });
    assert!(wrote.get());
    main.render_immediate(&mut NoOpMutations);
    assert_eq!(main.in_runtime(|| *value.peek()), 9);
}

// The host's side: a workspace with a pop-out asks the platform for a
// window, the pop-out's own dom draws its tabs, and closing it brings
// them back.

use architect_workspace::{
    Dest, FormClass, MemoryStore, Pane, PaneId, Popout, Screenset, Screensets, Store, Target, View,
    Views, WindowId, WindowSystem, Windows, Workspace, WorkspaceStore, WorkspaceView,
};

/// A platform that records what it was asked.
#[derive(Default)]
struct Fake {
    opened: RefCell<Vec<Popout>>,
    closed: RefCell<Vec<WindowId>>,
}

/// The fake, as the host holds it.
struct Platform(Rc<Fake>);

impl WindowSystem for Platform {
    fn open(&self, popout: Popout) {
        self.0.opened.borrow_mut().push(popout);
    }
    fn close(&self, window: &WindowId) {
        self.0.closed.borrow_mut().push(window.clone());
    }
    fn origin(&self, _: &WindowId) -> Option<(f32, f32)> {
        Some((0.0, 0.0))
    }
    fn window_at(&self, _: f32, _: f32) -> Option<WindowId> {
        None
    }
    fn on_release_outside(&self, _: &WindowId, _: Rc<dyn Fn(f32, f32)>) {}
}

thread_local! {
    static FAKE: RefCell<Option<Rc<Fake>>> = const { RefCell::new(None) };
    static WORKSPACE: RefCell<Option<Signal<Workspace>>> = const { RefCell::new(None) };
    static SETS: RefCell<Option<Signal<Screensets>>> = const { RefCell::new(None) };
    static STORE: RefCell<Option<MemoryStore>> = const { RefCell::new(None) };
    static DRAWN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static COMMANDS: RefCell<Option<architect_commands::Commands>> = const { RefCell::new(None) };
}

fn note(id: &str) -> Pane {
    Pane::new(id, "note", id)
}

#[component]
fn Note() -> Element {
    let pane = architect_workspace::use_pane();
    DRAWN.with(|d| d.borrow_mut().push(pane.id.0.clone()));
    rsx! { "{pane.id.0}" }
}

fn views() -> Views {
    Views::new().view(View::new("note", "Note", |_| rsx! { Note {} }).many())
}

fn app() -> Element {
    let fake = FAKE.with(|f| f.borrow().clone().expect("a fake platform"));
    use_context_provider(|| Windows::new(Platform(fake)));
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        w.open(note("a"), Target::Tab).unwrap();
        w.open(note("b"), Target::Tab).unwrap();
        w.move_pane(&PaneId::new("b"), Dest::Window(None)).unwrap();
        w
    });
    WORKSPACE.with(|s| *s.borrow_mut() = Some(workspace));
    let views = use_hook(views);
    rsx! { WorkspaceView { workspace, views } }
}

#[test]
fn a_popout_in_the_layout_opens_a_window_that_draws_its_tabs_and_gives_them_back() {
    FAKE.with(|f| *f.borrow_mut() = Some(Rc::new(Fake::default())));
    let fake = FAKE.with(|f| f.borrow().clone().unwrap());
    let mut main = VirtualDom::new(app);
    main.rebuild_in_place();
    let workspace = WORKSPACE.with(|s| s.borrow().unwrap());

    // The platform was asked for one window, for the pop-out.
    let popout = {
        let opened = fake.opened.borrow();
        assert_eq!(opened.len(), 1);
        opened[0].clone()
    };
    let id = main.in_runtime(|| workspace.peek().windows[1].id.clone());
    assert_eq!(*popout.id(), id);
    assert_eq!(popout.title(), "b");

    // Its own dom draws the tab that went there (and the main one doesn't).
    DRAWN.with(|d| d.borrow_mut().clear());
    let mut window = popout.dom();
    window.rebuild_in_place();
    assert_eq!(DRAWN.with(|d| d.borrow().clone()), ["b"]);

    // Moved: the layout keeps where.
    let at = architect_workspace::Rect::new(10.0, 20.0, 500.0, 400.0);
    window.in_runtime(|| popout.moved(at));
    assert_eq!(
        main.in_runtime(|| workspace.peek().windows[1].bounds),
        Some(at)
    );

    // Closed by the person: its tab is back in the main window.
    window.in_runtime(|| popout.closed());
    drop(window);
    main.render_immediate(&mut NoOpMutations);
    let back = main.in_runtime(|| {
        let w = workspace.peek();
        (
            w.windows.len(),
            w.locate(&PaneId::new("b")).map(|l| l.window),
        )
    });
    assert_eq!(back, (1, Some(0)));
    // It was the person's close: the host asked for nothing more.
    assert_eq!(fake.opened.borrow().len(), 1);
}

#[test]
fn a_popout_the_layout_drops_closes_its_window() {
    FAKE.with(|f| *f.borrow_mut() = Some(Rc::new(Fake::default())));
    let fake = FAKE.with(|f| f.borrow().clone().unwrap());
    let mut main = VirtualDom::new(app);
    main.rebuild_in_place();
    let workspace = WORKSPACE.with(|s| s.borrow().unwrap());
    let id = main.in_runtime(|| workspace.peek().windows[1].id.clone());
    // "Move to the main window" from a menu or a key.
    main.in_runtime(|| {
        let mut w = workspace;
        w.write().to_main_window(&PaneId::new("b")).unwrap();
    });
    main.render_immediate(&mut NoOpMutations);
    assert_eq!(*fake.closed.borrow(), [id]);
}

fn kept_app() -> Element {
    let commands = architect_commands::use_commands_provider();
    COMMANDS.with(|c| *c.borrow_mut() = Some(commands));
    let workspace = use_signal(Workspace::new);
    let sets = use_signal(|| {
        let mut mix = Workspace::new();
        mix.open(note("mixer"), Target::Tab).unwrap();
        Screensets::default()
            .with_builtins([Screenset::new("Mix").slot(2).layout(FormClass::Desk, mix)])
    });
    WORKSPACE.with(|s| *s.borrow_mut() = Some(workspace));
    SETS.with(|s| *s.borrow_mut() = Some(sets));
    let store =
        use_hook(|| WorkspaceStore::new(STORE.with(|s| s.borrow().clone().expect("a store"))));
    let views = use_hook(views);
    rsx! { WorkspaceView { workspace, views, screensets: sets, store } }
}

#[test]
fn the_kept_layout_comes_back_when_the_host_mounts_and_changes_are_kept() {
    // Kept from before: a live desk layout with one note.
    let store = MemoryStore::new();
    let mut before = Screensets::default();
    let mut live = Workspace::new();
    live.open(note("kept"), Target::Tab).unwrap();
    before.set_working(FormClass::Desk, live);
    architect_workspace::store::save(&store, &before);
    STORE.with(|s| *s.borrow_mut() = Some(store.clone()));

    let mut main = VirtualDom::new(kept_app);
    main.rebuild_in_place();
    let workspace = WORKSPACE.with(|s| s.borrow().unwrap());
    let sets = SETS.with(|s| s.borrow().unwrap());
    let open = main.in_runtime(|| workspace.peek().pane(&PaneId::new("kept")).is_some());
    assert!(open, "the kept layout is the live one");
    // The app's sets joined the kept ones.
    assert!(main.in_runtime(|| sets.peek().get("mix").is_some()));

    // A change settles: the store has it.
    main.in_runtime(|| {
        let mut w = workspace;
        w.write().open(note("new"), Target::Tab).unwrap();
    });
    // The redraws, then the effects (they run once nothing is dirty; the
    // host's own write as it settles takes one more turn).
    for _ in 0..2 {
        main.render_immediate(&mut NoOpMutations);
        main.process_events();
    }
    let kept = architect_workspace::store::load(&store, Vec::new());
    let has = kept
        .working(FormClass::Desk)
        .is_some_and(|w| w.pane(&PaneId::new("new")).is_some());
    assert!(
        has,
        "the store keeps the live layout: {:?}",
        store.load().map(|j| j.len())
    );
}

#[test]
fn command_2_recalls_the_mix_screenset() {
    STORE.with(|s| *s.borrow_mut() = Some(MemoryStore::new()));
    let mut main = VirtualDom::new(kept_app);
    main.rebuild_in_place();
    let workspace = WORKSPACE.with(|s| s.borrow().unwrap());
    let sets = SETS.with(|s| s.borrow().unwrap());
    let commands = COMMANDS.with(|c| c.borrow().unwrap());
    main.render_immediate(&mut NoOpMutations);
    let ran = main.in_runtime(|| commands.handle_chord("⌘2"));
    assert!(ran, "⌘2 is Recall Mix");
    main.render_immediate(&mut NoOpMutations);
    let (mixer, active) = main.in_runtime(|| {
        (
            workspace.peek().pane(&PaneId::new("mixer")).is_some(),
            sets.peek().active.clone(),
        )
    });
    assert!(mixer, "the Mix layout is the live one");
    assert_eq!(active.as_deref(), Some("mix"));
    // Nothing in slot 5: the chord does nothing.
    assert!(!main.in_runtime(|| commands.handle_chord("⌘5")));
}

thread_local! {
    static STATUS: RefCell<Option<Signal<Vec<architect_shell::StatusItem>>>> = const { RefCell::new(None) };
}

#[component]
fn Counted() -> Element {
    let pane = architect_workspace::use_pane();
    architect_workspace::use_pane_status(vec![architect_shell::StatusItem::new(
        "mine",
        pane.id.0.clone(),
    )]);
    rsx! { "{pane.id.0}" }
}

fn status_app() -> Element {
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        w.open(Pane::new("a", "counted", "a"), Target::Tab).unwrap();
        w.open(Pane::new("b", "counted", "b"), Target::Tab).unwrap();
        w
    });
    let status = use_signal(Vec::new);
    WORKSPACE.with(|s| *s.borrow_mut() = Some(workspace));
    STATUS.with(|s| *s.borrow_mut() = Some(status));
    let views = use_hook(|| {
        Views::new().view(View::new("counted", "Counted", |_| rsx! { Counted {} }).many())
    });
    rsx! { WorkspaceView { workspace, views, status } }
}

#[test]
fn a_panes_status_items_show_while_it_is_the_document_and_survive_a_redraw() {
    let mut main = VirtualDom::new(status_app);
    main.rebuild_in_place();
    let workspace = WORKSPACE.with(|s| s.borrow().unwrap());
    let status = STATUS.with(|s| s.borrow().unwrap());
    let shown = |main: &VirtualDom| {
        main.in_runtime(|| {
            status
                .peek()
                .iter()
                .map(|i| i.text.clone())
                .collect::<Vec<_>>()
        })
    };
    let settle = |main: &mut VirtualDom| {
        for _ in 0..3 {
            main.render_immediate(&mut NoOpMutations);
            main.process_events();
        }
    };
    settle(&mut main);
    assert_eq!(shown(&main), ["b"]);
    // Split: "b" is drawn afresh in a new group; its items stay.
    main.in_runtime(|| {
        let mut w = workspace;
        let mut ws = w.write();
        let g = ws.locate(&PaneId::new("b")).unwrap().group;
        ws.move_pane(
            &PaneId::new("b"),
            Dest::Edge {
                group: g,
                side: architect_workspace::Side::Bottom,
            },
        )
        .unwrap();
    });
    settle(&mut main);
    assert_eq!(shown(&main), ["b"]);
    // "a" becomes the document: its items show.
    main.in_runtime(|| {
        let mut w = workspace;
        w.write().activate(&PaneId::new("a")).unwrap();
    });
    settle(&mut main);
    assert_eq!(shown(&main), ["a"]);
}

// A dock popped out into a window of its own: through its actions, the
// platform opens a window for it whose dom draws the dock, and closing
// that window docks it back.

fn dock_app() -> Element {
    let fake = FAKE.with(|f| f.borrow().clone().expect("a fake platform"));
    use_context_provider(|| Windows::new(Platform(fake)));
    let commands = architect_commands::use_commands_provider();
    COMMANDS.with(|c| *c.borrow_mut() = Some(commands));
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        w.open(note("doc"), Target::Tab).unwrap();
        w.open(note("files"), Target::Dock(architect_workspace::Side::Left))
            .unwrap();
        w.open(
            note("search"),
            Target::Dock(architect_workspace::Side::Left),
        )
        .unwrap();
        w.activate(&PaneId::new("files")).unwrap();
        w.windows[0].docks.left.ribbon = Some(architect_workspace::Ribbon::default());
        w
    });
    WORKSPACE.with(|s| *s.borrow_mut() = Some(workspace));
    let views = use_hook(views);
    rsx! { WorkspaceView { workspace, views } }
}

/// Whether the command `id` is enabled now.
fn enabled(main: &VirtualDom, id: &str) -> bool {
    let commands = COMMANDS.with(|c| c.borrow().unwrap());
    main.in_runtime(|| commands.all())
        .into_iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no command {id}"))
        .enabled
}

#[test]
fn the_sidebar_pops_out_into_a_window_that_draws_it_and_docks_back() {
    use architect_workspace::Side;
    FAKE.with(|f| *f.borrow_mut() = Some(Rc::new(Fake::default())));
    let fake = FAKE.with(|f| f.borrow().clone().unwrap());
    let mut main = VirtualDom::new(dock_app);
    main.rebuild_in_place();
    main.render_immediate(&mut NoOpMutations);
    let workspace = WORKSPACE.with(|s| s.borrow().unwrap());
    let commands = COMMANDS.with(|c| c.borrow().unwrap());
    assert!(enabled(&main, "WORKSPACE_POP_OUT_LEFT_DOCK"));
    assert!(!enabled(&main, "WORKSPACE_DOCK_BACK_LEFT_DOCK"));
    assert!(
        !enabled(&main, "WORKSPACE_POP_OUT_RIGHT_DOCK"),
        "the right dock is empty"
    );

    // The action: the dock is in a window of its own.
    assert!(main.in_runtime(|| commands.run("WORKSPACE_POP_OUT_LEFT_DOCK")));
    main.render_immediate(&mut NoOpMutations);
    let out = main.in_runtime(|| workspace.peek().dock_window(Side::Left));
    assert_eq!(out, Some(1));
    let popout = {
        let opened = fake.opened.borrow();
        assert_eq!(opened.len(), 1);
        opened[0].clone()
    };
    assert_eq!(*popout.id(), WindowId::dock(Side::Left));
    assert_eq!(popout.title(), "Sidebar");
    assert!(enabled(&main, "WORKSPACE_DOCK_BACK_LEFT_DOCK"));
    assert!(!enabled(&main, "WORKSPACE_POP_OUT_LEFT_DOCK"));

    // Its own dom draws the dock's tab on show (and the main one doesn't).
    DRAWN.with(|d| d.borrow_mut().clear());
    let mut window = popout.dom();
    window.rebuild_in_place();
    assert_eq!(DRAWN.with(|d| d.borrow().clone()), ["files"]);
    DRAWN.with(|d| d.borrow_mut().clear());
    main.render_immediate(&mut NoOpMutations);
    assert!(
        !DRAWN.with(|d| d.borrow().contains(&"files".to_owned())),
        "the main window no longer draws the dock"
    );
    drop(window);

    // "Dock the sidebar back": its window closes.
    assert!(main.in_runtime(|| commands.run("WORKSPACE_DOCK_BACK_LEFT_DOCK")));
    main.render_immediate(&mut NoOpMutations);
    assert_eq!(*fake.closed.borrow(), [WindowId::dock(Side::Left)]);
    let back = main.in_runtime(|| {
        let w = workspace.peek();
        (
            w.windows.len(),
            w.locate(&PaneId::new("files")).map(|l| (l.window, l.area)),
            w.windows[0].docks.left.ribbon.is_some(),
        )
    });
    assert_eq!(
        back,
        (
            1,
            Some((0, architect_workspace::Area::Dock(Side::Left))),
            true
        )
    );

    // Out again, and the person closes its window: it docks back too.
    assert!(main.in_runtime(|| commands.run("WORKSPACE_POP_OUT_LEFT_DOCK")));
    main.render_immediate(&mut NoOpMutations);
    let popout = fake.opened.borrow().last().cloned().unwrap();
    let window = popout.dom();
    window.in_runtime(|| popout.closed());
    drop(window);
    main.render_immediate(&mut NoOpMutations);
    let back = main.in_runtime(|| {
        let w = workspace.peek();
        (w.windows.len(), w.dock_window(Side::Left))
    });
    assert_eq!(back, (1, None));
}

// A pane's state, typed: kept as its type's JSON, read back as the type,
// and `None` for a state that is not one.

#[derive(Debug, Clone, PartialEq, facet::Facet)]
struct Scroll {
    top: f32,
    zoom: u32,
}

thread_local! {
    static PANE: RefCell<Option<architect_workspace::PaneHandle>> = const { RefCell::new(None) };
}

#[component]
fn Scrolled() -> Element {
    let pane = architect_workspace::use_pane();
    PANE.with(|p| *p.borrow_mut() = Some(pane.clone()));
    rsx! { "{pane.id.0}" }
}

fn typed_app() -> Element {
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        let mut pane = Pane::new("s", "scrolled", "s");
        pane.state = "plain text".into();
        w.open(pane, Target::Tab).unwrap();
        w
    });
    WORKSPACE.with(|s| *s.borrow_mut() = Some(workspace));
    let views =
        use_hook(|| Views::new().view(View::new("scrolled", "Scrolled", |_| rsx! { Scrolled {} })));
    rsx! { WorkspaceView { workspace, views } }
}

#[test]
fn a_panes_state_is_kept_and_read_as_its_type() {
    let mut main = VirtualDom::new(typed_app);
    main.rebuild_in_place();
    let workspace = WORKSPACE.with(|s| s.borrow().unwrap());
    let pane = PANE.with(|p| p.borrow().clone().expect("the pane drew"));
    assert_eq!(main.in_runtime(|| pane.state_as::<Scroll>()), None);

    let kept = Scroll { top: 12.5, zoom: 3 };
    main.in_runtime(|| pane.set_state_as(&kept));
    main.render_immediate(&mut NoOpMutations);
    assert_eq!(main.in_runtime(|| pane.state_as::<Scroll>()), Some(kept));
    let raw = main.in_runtime(|| {
        workspace
            .peek()
            .pane(&PaneId::new("s"))
            .map(|p| p.state.clone())
    });
    assert_eq!(raw.as_deref(), Some(r#"{"top":12.5,"zoom":3}"#));
}

// Reset layout: whatever a person did to the windows (a dock popped out,
// a dock widened), the screenset's built-in layout comes back, in the
// main window, its pop-out closed (#353).

fn dock_default() -> Workspace {
    let mut w = Workspace::new();
    w.open(note("doc"), Target::Tab).unwrap();
    w.open(note("files"), Target::Dock(architect_workspace::Side::Left))
        .unwrap();
    w.activate(&PaneId::new("doc")).unwrap();
    w.windows[0].docks.left.ribbon = Some(architect_workspace::Ribbon::default());
    w
}

fn reset_app() -> Element {
    let fake = FAKE.with(|f| f.borrow().clone().expect("a fake platform"));
    use_context_provider(|| Windows::new(Platform(fake)));
    let commands = architect_commands::use_commands_provider();
    COMMANDS.with(|c| *c.borrow_mut() = Some(commands));
    let workspace = use_signal(dock_default);
    // Built in, never recalled: the app starts on it with no set active.
    let sets = use_signal(|| {
        Screensets::default()
            .with_builtins([Screenset::new("Write").layout(FormClass::Desk, dock_default())])
    });
    WORKSPACE.with(|s| *s.borrow_mut() = Some(workspace));
    SETS.with(|s| *s.borrow_mut() = Some(sets));
    let views = use_hook(views);
    rsx! { WorkspaceView { workspace, views, screensets: sets, form: FormClass::Desk } }
}

/// The redraws, then the effects (the host keeps the live layout in one).
fn settle(main: &mut VirtualDom) {
    for _ in 0..3 {
        main.render_immediate(&mut NoOpMutations);
        main.process_events();
    }
}

#[test]
fn reset_layout_docks_a_popped_out_sidebar_back_at_its_saved_width() {
    use architect_workspace::Side;
    FAKE.with(|f| *f.borrow_mut() = Some(Rc::new(Fake::default())));
    let fake = FAKE.with(|f| f.borrow().clone().unwrap());
    let mut main = VirtualDom::new(reset_app);
    main.rebuild_in_place();
    settle(&mut main);
    let mut workspace = WORKSPACE.with(|s| s.borrow().unwrap());
    let commands = COMMANDS.with(|c| c.borrow().unwrap());
    let width = dock_default().windows[0].docks.left.size;
    assert!(
        !enabled(&main, "WORKSPACE_RESET_LAYOUT"),
        "nothing to reset yet"
    );

    // Out, and wider.
    assert!(main.in_runtime(|| commands.run("WORKSPACE_POP_OUT_LEFT_DOCK")));
    settle(&mut main);
    main.in_runtime(|| {
        let mut w = workspace.write();
        for window in &mut w.windows {
            window.docks.left.size = width + 300.0;
        }
    });
    settle(&mut main);
    assert_eq!(
        main.in_runtime(|| workspace.peek().dock_window(Side::Left)),
        Some(1)
    );
    assert!(enabled(&main, "WORKSPACE_RESET_LAYOUT"));

    // Reset: one window, the sidebar home at its width, the pop-out closed.
    assert!(main.in_runtime(|| commands.run("WORKSPACE_RESET_LAYOUT")));
    settle(&mut main);
    let (windows, out, size, same) = main.in_runtime(|| {
        let w = workspace.peek();
        (
            w.windows.len(),
            w.dock_window(Side::Left),
            w.windows[0].docks.left.size,
            w.same_arrangement(&dock_default()),
        )
    });
    assert_eq!((windows, out), (1, None));
    assert!((size - width).abs() < f32::EPSILON, "{size} != {width}");
    assert!(same, "the layout is the default again");
    assert_eq!(*fake.closed.borrow(), [WindowId::dock(Side::Left)]);
    let sets = SETS.with(|s| s.borrow().unwrap());
    assert!(main.in_runtime(|| sets.peek().active.is_some()));
    assert!(!enabled(&main, "WORKSPACE_RESET_LAYOUT"));
}
