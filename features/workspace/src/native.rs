//! A desk app's launcher (feature `native`): dioxus-native's, plus the
//! workspace's pop-out windows as real OS windows.
//!
//! dioxus-native opens one window, at start. [`launch`] runs its
//! application inside one of its own that opens more while the app runs:
//! each pop-out is a Blitz view of its own (a document, a `VirtualDom`,
//! a renderer) driven here, its root the workspace host's
//! [`Popout`](crate::Popout) reading the main window's
//! `Signal<Workspace>`. The main window is dioxus-native's untouched
//! (its window events, its key bindings, hot reload).
//!
//! What a pop-out does not have: dioxus-native's window-event hooks
//! (`use_window_event` is the main window's), so the app's chords
//! (`architect_commands::WindowKeys`) are heard in the main window only;
//! keys in a pop-out reach what has the focus there.
//!
//! The main window is in context as a [`MainWindow`]: an app that keeps
//! running with no window (`architect-presence`'s background mode) says
//! what closing it does, and quits the loop itself.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

use blitz_shell::{BlitzShellEvent, BlitzShellProxy, View, WindowConfig};
use blitz_traits::net::{NetHandler, NetProvider, Request};
use dioxus::core::ScopeId;
use dioxus::prelude::*;
use dioxus_native::winit::application::ApplicationHandler;
use dioxus_native::winit::dpi::{LogicalPosition, LogicalSize};
use dioxus_native::winit::event::{ElementState, StartCause, WindowEvent};
use dioxus_native::winit::event_loop::ActiveEventLoop;
use dioxus_native::winit::window::{Window as OsWindow, WindowAttributes, WindowId as OsId};
use dioxus_native::{
    DioxusDocument, DioxusNativeApplication, DioxusNativeEvent, DioxusNativeWindowRenderer,
    DocumentConfig,
};

#[cfg(target_os = "macos")]
use dioxus_native::winit::platform::macos::ApplicationHandlerExtMacOS;

use crate::host::{Popout, WindowSystem, Windows};
use crate::model::{Rect, WindowId};

/// Run `app` in a window made with `window`, pop-outs opening beside it.
/// What `dioxus_native::launch_cfg` does for one window, with a
/// [`Windows`] in context for every workspace host.
pub fn launch(app: fn() -> Element, window: WindowAttributes) {
    launch_with_fonts(app, window, &[]);
}

/// [`launch`], every window's text able to use the faces in `fonts`
/// (font files, each family by the name in it) beside the system's.
///
/// An app's own face goes here: Blitz does not load an `@font-face`
/// whose source is a `data:` URI, and has no file to read beside a
/// bundled app.
pub fn launch_with_fonts(
    app: fn() -> Element,
    window: WindowAttributes,
    fonts: &'static [&'static [u8]],
) {
    let event_loop = blitz_shell::create_default_event_loop();
    let winit_proxy = event_loop.create_proxy();
    // The main window's events go to dioxus-native's queue; the pop-outs'
    // to this one, so neither drops the other's.
    let (proxy, queue) = BlitzShellProxy::new(winit_proxy.clone());
    let (own, own_queue) = BlitzShellProxy::new(winit_proxy);
    let desk = Rc::new(Desk {
        wake: own.clone(),
        requests: RefCell::new(Vec::new()),
        main: RefCell::new(None),
        open: RefCell::new(Vec::new()),
        releases: RefCell::new(Vec::new()),
        on_close: RefCell::new(None),
    });

    #[cfg(all(feature = "hot-reload", debug_assertions))]
    {
        let proxy = proxy.clone();
        dioxus_devtools::connect(move |event| {
            let event = dioxus_native::DioxusNativeEvent::DevserverEvent(event);
            proxy.send_event(BlitzShellEvent::embedder_event(event));
        });
    }

    let vdom = VirtualDom::new_with_props(
        Root,
        RootProps {
            app,
            windows: Windows(desk.clone()),
            desk: DeskHandle(desk.clone()),
        },
    );
    let doc = document(vdom, &proxy, fonts);
    let config =
        WindowConfig::with_attributes(Box::new(doc), DioxusNativeWindowRenderer::new(), window);
    let app = App {
        inner: DioxusNativeApplication::new(proxy, queue, config),
        desk,
        popouts: HashMap::new(),
        own,
        own_queue,
        fonts,
    };
    // An event loop that cannot run has no window to say so in; the
    // process ends either way.
    let _ = event_loop.run_app(app);
}

/// A document for `vdom`: the bundle's assets and `data:` URIs load, and
/// the app's `fonts` join the system's.
fn document(
    vdom: VirtualDom,
    proxy: &BlitzShellProxy,
    fonts: &'static [&'static [u8]],
) -> DioxusDocument {
    let waker: Arc<dyn blitz_traits::net::NetWaker> = Arc::new(proxy.clone());
    let net: Arc<dyn NetProvider> = Arc::new(Assets {
        data: blitz_shell::DataUriNetProvider::shared(Some(waker)),
    });
    vdom.provide_root_context(Arc::clone(&net));
    DioxusDocument::new(
        vdom,
        DocumentConfig {
            net_provider: Some(net),
            font_ctx: (!fonts.is_empty()).then(|| font_context(fonts)),
            ..Default::default()
        },
    )
}

/// The system's faces and `fonts`.
fn font_context(fonts: &[&[u8]]) -> dioxus_native::FontContext {
    let mut ctx = dioxus_native::FontContext::default();
    for font in fonts {
        ctx.collection.register_fonts(font.to_vec().into(), None);
    }
    ctx
}

/// Fetches `dioxus:` assets from the bundle, the rest as `data:` URIs.
struct Assets {
    data: Arc<dyn NetProvider>,
}

impl NetProvider for Assets {
    fn fetch(&self, doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        if request.url.scheme() == "dioxus" {
            if let Ok(found) = dioxus_asset_resolver::native::serve_asset(request.url.path()) {
                handler.bytes(request.url.to_string(), found.into_body().into());
            }
        } else {
            self.data.fetch(doc_id, request, handler);
        }
    }
}

/// The desk's windows, as the hosts see them: asked to open or close one,
/// it queues the ask and wakes the event loop (a window is made where the
/// loop is at hand).
struct Desk {
    wake: BlitzShellProxy,
    requests: RefCell<Vec<Ask>>,
    main: RefCell<Option<Arc<dyn OsWindow>>>,
    /// The pop-outs open now, by id: for where they are.
    open: RefCell<Vec<(WindowId, Arc<dyn OsWindow>)>>,
    /// Each window's host, told where a drag let go outside it.
    releases: RefCell<Vec<(WindowId, Rc<dyn Fn(f32, f32)>)>>,
    /// Asked when the person closes the main window ([`MainWindow::on_close`]).
    on_close: RefCell<Option<Rc<dyn Fn() -> bool>>>,
}

impl Desk {
    /// The workspace's id for the OS window `os`.
    fn id_of(&self, os: OsId) -> Option<WindowId> {
        if self.main.borrow().as_ref().is_some_and(|w| w.id() == os) {
            return Some(WindowId::main());
        }
        self.open
            .borrow()
            .iter()
            .find(|(_, w)| w.id() == os)
            .map(|(id, _)| id.clone())
    }

    /// A pointer's button went up in `os` at `position`: outside its
    /// content, its host hears where (the document there does not).
    fn released(&self, os: OsId, position: (f64, f64)) {
        let Some(id) = self.id_of(os) else {
            return;
        };
        let window = if id == WindowId::main() {
            self.main.borrow().clone()
        } else {
            self.open
                .borrow()
                .iter()
                .find(|(w, _)| *w == id)
                .map(|(_, w)| Arc::clone(w))
        };
        let Some(window) = window else {
            return;
        };
        let scale = window.scale_factor();
        let size = window.surface_size();
        let (x, y) = position;
        let inside =
            x >= 0.0 && y >= 0.0 && x <= f64::from(size.width) && y <= f64::from(size.height);
        if inside {
            return;
        }
        let hear = self
            .releases
            .borrow()
            .iter()
            .rev()
            .find(|(w, _)| *w == id)
            .map(|(_, f)| Rc::clone(f));
        if let Some(hear) = hear {
            hear(narrow(x / scale), narrow(y / scale));
        }
    }
}

enum Ask {
    Open(Box<Popout>),
    Close(WindowId),
    /// End the event loop: the app quits.
    Quit,
}

/// The desk's main window, in context for everything in it: the OS
/// window, what closing it does, and quitting.
///
/// Closing the last window quits, as dioxus-native has it, unless the app
/// says otherwise ([`Self::on_close`]): a menu-bar app hides the window
/// and keeps running, and quits with [`Self::quit`].
#[derive(Clone)]
pub struct MainWindow(Rc<Desk>);

impl MainWindow {
    /// The OS window, once it is open.
    #[must_use]
    pub fn window(&self) -> Option<Arc<dyn OsWindow>> {
        self.0.main.borrow().clone()
    }

    /// Asks `close` each time the person closes the main window (its
    /// close button, Window › Close): `true` keeps the window (the app hid
    /// it), `false` lets it close, which quits when it is the last. `None`
    /// puts the default back. Called on the event loop's thread, outside
    /// any dom: change no signal from it.
    pub fn on_close(&self, close: Option<Rc<dyn Fn() -> bool>>) {
        *self.0.on_close.borrow_mut() = close;
    }

    /// Ends the event loop on its next turn: the app quits, its windows
    /// with it, hidden or not.
    pub fn quit(&self) {
        self.0.requests.borrow_mut().push(Ask::Quit);
        self.0.wake.wake_up();
    }
}

impl PartialEq for MainWindow {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

/// Where `window`'s content is on screen, logical points.
fn content_rect(window: &dyn OsWindow) -> Option<Rect> {
    let scale = window.scale_factor();
    let outer = window.outer_position().ok()?;
    let inset = window.surface_position();
    let size = window.surface_size();
    let to = |v: f64| narrow(v / scale);
    Some(Rect::new(
        to(f64::from(outer.x) + f64::from(inset.x)),
        to(f64::from(outer.y) + f64::from(inset.y)),
        to(f64::from(size.width)),
        to(f64::from(size.height)),
    ))
}

impl WindowSystem for Desk {
    fn open(&self, popout: Popout) {
        self.requests.borrow_mut().push(Ask::Open(Box::new(popout)));
        self.wake.wake_up();
    }

    fn close(&self, window: &WindowId) {
        self.requests.borrow_mut().push(Ask::Close(window.clone()));
        self.wake.wake_up();
    }

    fn origin(&self, window: &WindowId) -> Option<(f32, f32)> {
        self.content(window).map(|r| (r.x, r.y))
    }

    fn content(&self, window: &WindowId) -> Option<Rect> {
        if *window == WindowId::main() {
            self.main
                .borrow()
                .as_ref()
                .and_then(|w| content_rect(w.as_ref()))
        } else {
            self.open
                .borrow()
                .iter()
                .find(|(id, _)| id == window)
                .and_then(|(_, w)| content_rect(w.as_ref()))
        }
    }

    fn on_release_outside(&self, window: &WindowId, release: Rc<dyn Fn(f32, f32)>) {
        self.releases.borrow_mut().push((window.clone(), release));
    }

    fn window_at(&self, x: f32, y: f32) -> Option<WindowId> {
        let hit = |w: &Arc<dyn OsWindow>| {
            content_rect(w.as_ref()).is_some_and(|r| crate::drop::contains(r, x, y))
        };
        let popout = self
            .open
            .borrow()
            .iter()
            .rev()
            .find(|(_, w)| hit(w))
            .map(|(id, _)| id.clone());
        popout.or_else(|| {
            self.main
                .borrow()
                .as_ref()
                .filter(|w| hit(w))
                .map(|_| WindowId::main())
        })
    }
}

/// The desk, as a prop (compared by identity).
#[derive(Clone)]
struct DeskHandle(Rc<Desk>);

impl PartialEq for DeskHandle {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }
}

#[derive(Props, Clone, PartialEq)]
struct RootProps {
    app: fn() -> Element,
    windows: Windows,
    desk: DeskHandle,
}

/// The main window's root: the [`Windows`] and the [`MainWindow`] in
/// context, the window itself known to the desk, then the app.
#[allow(non_snake_case, clippy::needless_pass_by_value)]
fn Root(props: RootProps) -> Element {
    use_context_provider(|| props.windows.clone());
    use_context_provider(|| MainWindow(props.desk.0.clone()));
    let window = dioxus_native::use_window();
    let desk = props.desk.clone();
    use_hook(move || {
        *desk.0.main.borrow_mut() = Some(window);
    });
    (props.app)()
}

/// A pop-out on screen: its view, and the host's handle on it.
struct Open {
    popout: Popout,
    view: View<DioxusNativeWindowRenderer>,
}

/// dioxus-native's application, and the pop-outs beside its window.
struct App {
    inner: DioxusNativeApplication,
    desk: Rc<Desk>,
    popouts: HashMap<OsId, Open>,
    own: BlitzShellProxy,
    own_queue: Receiver<BlitzShellEvent>,
    /// The app's faces, for each pop-out's document.
    fonts: &'static [&'static [u8]],
}

impl App {
    /// Do what the hosts asked: open and close windows.
    fn asks(&mut self, event_loop: &dyn ActiveEventLoop) {
        let asks = std::mem::take(&mut *self.desk.requests.borrow_mut());
        for ask in asks {
            match ask {
                Ask::Open(popout) => self.open(event_loop, *popout),
                Ask::Quit => event_loop.exit(),
                Ask::Close(id) => {
                    let os = self
                        .popouts
                        .iter()
                        .find(|(_, o)| *o.popout.id() == id)
                        .map(|(os, _)| *os);
                    if let Some(os) = os {
                        self.forget(os);
                    }
                }
            }
        }
    }

    fn open(&mut self, event_loop: &dyn ActiveEventLoop, popout: Popout) {
        if let Some(open) = self.popouts.values().find(|o| o.popout.id() == popout.id()) {
            open.view.window.focus_window();
            return;
        }
        let mut attributes = WindowAttributes::default().with_title(popout.title());
        let (w, h) = popout.bounds().map_or((640.0, 480.0), |b| (b.w, b.h));
        attributes = attributes.with_surface_size(LogicalSize::new(f64::from(w), f64::from(h)));
        if let Some(b) = popout.bounds() {
            attributes =
                attributes.with_position(LogicalPosition::new(f64::from(b.x), f64::from(b.y)));
        }
        let doc = document(popout.dom(), &self.own, self.fonts);
        let config = WindowConfig::with_attributes(
            Box::new(doc),
            DioxusNativeWindowRenderer::new(),
            attributes,
        );
        let mut view = View::init(config, event_loop, &self.own);
        let window = Arc::clone(&view.window);
        let doc = view.downcast_doc_mut::<DioxusDocument>();
        let shell = doc.inner.borrow().shell_provider.clone();
        let head = Head {
            window: Arc::clone(&window),
            os: window.id(),
            proxy: self.own.clone(),
        };
        let os_window = Arc::clone(&window);
        doc.vdom.in_scope(ScopeId::ROOT, move || {
            let head: Rc<dyn dioxus::document::Document> = Rc::new(head);
            provide_context(head);
            provide_context(shell);
            provide_context(os_window);
        });
        doc.initial_build();
        view.resume();
        view.request_redraw();
        self.desk
            .open
            .borrow_mut()
            .push((popout.id().clone(), Arc::clone(&window)));
        self.popouts.insert(view.window_id(), Open { popout, view });
    }

    /// The person closed the main window: whether the app kept it
    /// ([`MainWindow::on_close`]).
    fn kept_main(&self) -> bool {
        let close = self.desk.on_close.borrow().clone();
        close.is_some_and(|close| close())
    }

    /// Drop the pop-out's window (its document goes with it).
    fn forget(&mut self, os: OsId) {
        if let Some(open) = self.popouts.remove(&os) {
            let id = open.popout.id().clone();
            self.desk.open.borrow_mut().retain(|(w, _)| *w != id);
            self.desk.releases.borrow_mut().retain(|(w, _)| *w != id);
            drop(open);
        }
    }

    /// The pop-out `os` has an event from the platform.
    fn popout_event(&mut self, os: OsId, event: WindowEvent) {
        let Some(open) = self.popouts.get_mut(&os) else {
            return;
        };
        match &event {
            WindowEvent::CloseRequested => {
                let popout = open.popout.clone();
                let doc = open.view.downcast_doc_mut::<DioxusDocument>();
                doc.vdom.in_runtime(|| popout.closed());
                self.forget(os);
                return;
            }
            WindowEvent::Moved(_) | WindowEvent::SurfaceResized(_) => {
                if let Some(bounds) = content_rect(open.view.window.as_ref()) {
                    let popout = open.popout.clone();
                    let doc = open.view.downcast_doc_mut::<DioxusDocument>();
                    doc.vdom.in_runtime(|| popout.moved(bounds));
                }
            }
            _ => {}
        }
        open.view.handle_winit_event(event);
        self.own.send_event(BlitzShellEvent::Poll { window_id: os });
    }

    /// What the pop-outs' documents asked of the loop.
    fn own_events(&mut self) {
        while let Ok(event) = self.own_queue.try_recv() {
            match event {
                BlitzShellEvent::Poll { window_id } => {
                    if let Some(open) = self.popouts.get_mut(&window_id) {
                        open.view.poll();
                    }
                }
                BlitzShellEvent::RequestRedraw { doc_id } => {
                    if let Some(open) = self
                        .popouts
                        .values_mut()
                        .find(|o| o.view.doc.id() == doc_id)
                    {
                        open.view.frame_now();
                    }
                }
                BlitzShellEvent::ResumeReady { window_id } => {
                    if let Some(open) = self.popouts.get_mut(&window_id) {
                        let _ = open.view.complete_resume();
                    }
                }
                BlitzShellEvent::CloseWindow { window_id } => {
                    self.popout_event(window_id, WindowEvent::CloseRequested);
                }
                BlitzShellEvent::Embedder(event) => {
                    if let Some(DioxusNativeEvent::CreateHeadElement {
                        window,
                        name,
                        attributes,
                        contents,
                    }) = event.downcast_ref::<DioxusNativeEvent>()
                    {
                        if let Some(open) = self.popouts.get_mut(window) {
                            let doc = open.view.downcast_doc_mut::<DioxusDocument>();
                            doc.create_head_element(name, attributes, contents);
                            open.view.poll();
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

impl ApplicationHandler for App {
    #[cfg(target_os = "macos")]
    fn macos_handler(&mut self) -> Option<&mut dyn ApplicationHandlerExtMacOS> {
        self.inner.macos_handler()
    }

    fn resumed(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.inner.resumed(event_loop);
    }

    fn suspended(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn destroy_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        for open in self.popouts.values_mut() {
            open.view.suspend();
        }
        self.inner.destroy_surfaces(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.inner.about_to_wait(event_loop);
    }

    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.inner.can_create_surfaces(event_loop);
        for open in self.popouts.values_mut() {
            open.view.resume();
        }
        self.asks(event_loop);
    }

    fn new_events(&mut self, event_loop: &dyn ActiveEventLoop, cause: StartCause) {
        self.inner.new_events(event_loop, cause);
    }

    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, window: OsId, event: WindowEvent) {
        if let WindowEvent::PointerButton {
            state: ElementState::Released,
            primary: true,
            position,
            ..
        } = &event
        {
            self.desk.released(window, (position.x, position.y));
        }
        if self.popouts.contains_key(&window) {
            self.popout_event(window, event);
            self.asks(event_loop);
        } else if matches!(event, WindowEvent::CloseRequested) && self.kept_main() {
            // The app hid the window instead: the loop runs on.
            self.asks(event_loop);
        } else {
            self.inner.window_event(event_loop, window, event);
        }
    }

    fn proxy_wake_up(&mut self, event_loop: &dyn ActiveEventLoop) {
        self.own_events();
        self.asks(event_loop);
        self.inner.proxy_wake_up(event_loop);
        // The main window's turn may have asked for more.
        self.asks(event_loop);
    }
}

/// A pop-out's document head: its title is the OS window's; its styles
/// and links go into its document (as dioxus-native's do for its window),
/// on the loop's next turn.
struct Head {
    window: Arc<dyn OsWindow>,
    os: OsId,
    proxy: BlitzShellProxy,
}

impl dioxus::document::Document for Head {
    fn eval(&self, js: String) -> dioxus::document::Eval {
        dioxus::document::NoOpDocument.eval(js)
    }

    fn set_title(&self, title: String) {
        self.window.set_title(&title);
    }

    fn create_head_element(
        &self,
        name: &str,
        attributes: &[(&str, String)],
        contents: Option<String>,
    ) {
        if name == "title" {
            if let Some(title) = &contents {
                self.window.set_title(title);
            }
            return;
        }
        self.proxy.send_event(BlitzShellEvent::embedder_event(
            DioxusNativeEvent::CreateHeadElement {
                window: self.os,
                name: name.to_owned(),
                attributes: attributes
                    .iter()
                    .map(|(k, v)| ((*k).to_owned(), v.clone()))
                    .collect(),
                contents,
            },
        ));
    }

    fn create_head_component(&self) -> bool {
        true
    }
}

/// A screen length as the model's points (screens fit an `f32`).
#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}
