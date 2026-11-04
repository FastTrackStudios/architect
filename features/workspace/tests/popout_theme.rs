//! A pop-out window follows the app's theme: its chrome (the host's
//! `ShellTheme`) and the theme's CSS variables (the main window's `:root`
//! sheet, `architect_theme::RootCss`) change in the pop-out's own document
//! when they change in the main window, not only when it opens.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use architect_shell::ShellTheme;
use architect_theme::RootCss;
use architect_workspace::{
    Dest, Pane, PaneId, Popout, Target, View, Views, WindowId, WindowSystem, Windows, Workspace,
    WorkspaceView,
};
use blitz_dom::Document as _;
use dioxus::core::NoOpMutations;
use dioxus::prelude::*;
use dioxus_native::DioxusDocument;

/// A platform that keeps the pop-out it is asked to open.
struct Platform(Rc<RefCell<Vec<Popout>>>);

impl WindowSystem for Platform {
    fn open(&self, popout: Popout) {
        self.0.borrow_mut().push(popout);
    }
    fn close(&self, _: &WindowId) {}
    fn origin(&self, _: &WindowId) -> Option<(f32, f32)> {
        Some((0.0, 0.0))
    }
    fn window_at(&self, _: f32, _: f32) -> Option<WindowId> {
        None
    }
    fn on_release_outside(&self, _: &WindowId, _: Rc<dyn Fn(f32, f32)>) {}
}

thread_local! {
    static OPENED: Rc<RefCell<Vec<Popout>>> = Rc::default();
    static THEME: RefCell<Option<(Signal<ShellTheme>, Signal<String>)>> = const { RefCell::new(None) };
}

fn shell_theme(desk: &str) -> ShellTheme {
    ShellTheme {
        desk: desk.into(),
        ..ShellTheme::default()
    }
}

fn app() -> Element {
    use_context_provider(|| Windows::new(Platform(OPENED.with(Rc::clone))));
    let theme = use_signal(|| shell_theme("rgb(1, 2, 3)"));
    let root = use_context_provider(|| {
        RootCss(Signal::new(
            ":root { --background: rgb(10, 20, 30); }".to_owned(),
        ))
    })
    .0;
    THEME.with(|t| *t.borrow_mut() = Some((theme, root)));
    let workspace = use_signal(|| {
        let mut w = Workspace::new();
        w.open(Pane::new("a", "note", "a"), Target::Tab).unwrap();
        w.open(Pane::new("b", "note", "b"), Target::Tab).unwrap();
        w.move_pane(&PaneId::new("b"), Dest::Window(None)).unwrap();
        w
    });
    let views =
        use_hook(|| Views::new().view(View::new("note", "Note", |_| rsx! { "note" }).many()));
    rsx! { WorkspaceView { workspace, views, theme: theme() } }
}

/// The pop-out root's `style` attribute.
fn root_style(doc: &DioxusDocument) -> String {
    let inner = doc.inner.borrow();
    let root = inner
        .query_selector("div[tabindex]")
        .unwrap()
        .expect("the pop-out's root");
    inner
        .get_node(root)
        .and_then(|n| n.element_data())
        .and_then(|e| e.attr(blitz_dom::local_name!("style")))
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_popout_follows_theme_changes_after_it_opens() {
    let mut main = VirtualDom::new(app);
    main.rebuild_in_place();
    let popout = OPENED
        .with(|o| o.borrow().first().cloned())
        .expect("a pop-out opened");
    let mut window = DioxusDocument::new(
        popout.dom(),
        architect_workspace::native::document_config(None, &[]),
    );
    window.initial_build();
    let opened = root_style(&window);
    assert!(
        opened.contains("rgb(1, 2, 3)"),
        "its chrome, as it opens: {opened}"
    );
    assert!(
        opened.contains("--background: rgb(10, 20, 30)"),
        "the theme's variables, as it opens: {opened}"
    );

    // THEME_SET in the main window: the host's colours and the sheet change.
    let (theme, root) = THEME.with(|t| t.borrow().expect("the app ran"));
    main.in_runtime(|| {
        let (mut theme, mut root) = (theme, root);
        theme.set(shell_theme("rgb(200, 201, 202)"));
        root.set(":root { --background: rgb(240, 241, 242); }".to_owned());
    });
    main.render_immediate(&mut NoOpMutations);
    window.poll(None);
    let changed = root_style(&window);
    assert!(
        changed.contains("rgb(200, 201, 202)"),
        "its chrome follows: {changed}"
    );
    assert!(
        changed.contains("--background: rgb(240, 241, 242)"),
        "the theme's variables follow: {changed}"
    );
}
