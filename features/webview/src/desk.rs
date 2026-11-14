//! The desk's web view host (feature `desk`): a wry child web view of the
//! pane's OS window, clipped to the pane's rect.
//!
//! - **Where it is.** Each pane draws a holder element; a loop on the
//!   window's thread reads the holder's rect every frame and moves the
//!   native view there (`set_bounds`, ~0.15 ms), so it follows splits,
//!   docks, floats and window resizes. A pop-out's pane uses the pop-out's
//!   own window (its `Arc<dyn Window>` in context).
//! - **When it is shown.** Its tab in front and its pane sized, and
//!   nothing of the app's drawn over it: the loop hit-tests points of the
//!   pane's rect in the window's document (`LiveDocument::covered_at`) and
//!   hides the native view while ⌘K, a menu, a popover or a dialog covers
//!   any of them, as no native view can be under the app's own drawing.
//! - **What it loads.** Only its package, over the `architect-ext`
//!   protocol (`architect_extensions::webview::serve`, under its CSP); a
//!   navigation anywhere else, and every new window, is refused.
//! - **What it may do.** Its messages go to a
//!   `architect_extensions::webview::Bridge`: each call checked against the
//!   extension's capabilities and answered as it; the events it listens
//!   for come back the same way.
//! - **Keys.** The native view has the keyboard while it is focused, so
//!   typed keys stay in it; an Escape it did not take gives the focus back
//!   to the app (`focus_parent`).
//! - **Its life.** A view whose tab is switched away is kept, hidden, for
//!   [`PARKED`], so its state survives a tab switch; a pane's preview (a
//!   phone's switcher) draws a still placeholder, never a second view.
//!   When its `src` file changes (a development copy rebuilt), it reloads.
//!
//! Linux: `WebKitGTK` embeds a child only in an X11 window. Under Wayland
//! (the spike in `docs/content/architecture/embedded-views.md`, "Web
//! views") the pane says so and how to run Architect under `XWayland`
//! (`ARCHITECT_WEBVIEW_X11=1`).

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use architect_extensions::host::Host;
use architect_extensions::interface::{WebAsk, WebViews};
use architect_extensions::webview::{self, Bridge, Outbound, Shown};
use architect_workspace::native::MainWindow;
use architect_workspace::shot::LiveDocument;
use architect_workspace::use_pane;
use dioxus::prelude::*;
use dioxus_native::winit::window::Window as OsWindow;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::http::{Response, StatusCode};
use wry::{NewWindowResponse, Rect, WebView, WebViewBuilder};

/// How often a pane's native view is moved to its holder and its
/// messages delivered.
pub const FRAME: Duration = Duration::from_millis(16);

/// How long a view whose pane went away is kept, hidden, for its pane to
/// come back (a tab switch).
pub const PARKED: Duration = Duration::from_secs(60);

/// How far inside its pane's edges a view's overlap probes are, points.
const INSET: f64 = 24.0;

/// The attribute a pane's holder carries, its value the view's key.
const HOLDER: &str = "data-webview";

/// Provides [`WebViews`] for the app's window, drawing every `web` view
/// with a native web view over `host`'s extensions. Call it once, near
/// the app's root.
#[allow(clippy::must_use_candidate)]
pub fn use_desk_web_views(host: Host) -> WebViews {
    let provided = use_hook(move || {
        WebViews(Rc::new(move |ask: &WebAsk| {
            let host = HostRef(host.clone());
            let ask = ask.clone();
            rsx! { DeskWebPane { ask, host } }
        }))
    });
    use_context_provider(|| provided.clone());
    provided
}

/// The extensions' host, as a prop (one per app: always equal).
#[derive(Clone)]
pub struct HostRef(pub Host);

impl PartialEq for HostRef {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// One native view and its bridge.
struct Native {
    view: WebView,
    bridge: Bridge,
    /// What the view said, from its IPC handler.
    inbox: mpsc::Receiver<String>,
    /// What the host says to it, from any thread.
    outbox: mpsc::Receiver<Outbound>,
    bounds: Option<(i64, i64, i64, i64)>,
    visible: bool,
    theme: String,
    escape: Arc<AtomicBool>,
    parked: Option<Instant>,
    /// Its `src` file and when it last changed: a rebuild of a
    /// development copy reloads the view.
    src: Option<std::path::PathBuf>,
    changed: Option<std::time::SystemTime>,
    checked: Instant,
}

fn modified(path: Option<&std::path::Path>) -> Option<std::time::SystemTime> {
    std::fs::metadata(path?).and_then(|m| m.modified()).ok()
}

thread_local! {
    /// The window thread's native views, by key (`<extension>/<view>#<pane>`).
    static VIEWS: RefCell<HashMap<String, Native>> = RefCell::new(HashMap::new());
}

/// The URL a view opens for `path` in extension `id`'s package (`WebView2`
/// serves a custom protocol as `http://<scheme>.<host>/`).
fn view_url(id: &str, path: &str) -> String {
    if cfg!(windows) {
        format!(
            "http://{}.{id}/{}",
            webview::SCHEME,
            path.trim_start_matches('/')
        )
    } else {
        webview::url(id, path)
    }
}

/// Whether a navigation may happen: only within its package.
fn may_navigate(url: &str) -> bool {
    url.starts_with(&format!("{}://", webview::SCHEME))
        || url.starts_with(&format!("http://{}.", webview::SCHEME))
        || url == "about:blank"
}

#[cfg(target_os = "linux")]
fn pump() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if let Err(e) = gtk::init() {
            tracing::warn!(error = %e, "gtk did not start: web views cannot be shown");
        }
    });
    while gtk::events_pending() {
        gtk::main_iteration_do(false);
    }
}

#[cfg(not(target_os = "linux"))]
const fn pump() {}

/// Why a native view could not be made, in words for the pane.
fn why_not(e: &wry::Error) -> String {
    if cfg!(target_os = "linux") && matches!(e, wry::Error::UnsupportedWindowHandle) {
        "Web views need an X11 window on Linux: WebKitGTK cannot be a child of a Wayland \
         window. Start Architect with ARCHITECT_WEBVIEW_X11=1 to run it under XWayland."
            .to_owned()
    } else {
        format!("The web view could not start: {e}")
    }
}

/// Makes the native view for `ask` in `window`.
fn create(
    host: &Host,
    ask: &WebAsk,
    window: &dyn OsWindow,
    rect: Rect,
) -> Result<Native, String> {
    pump();
    let dir = ask
        .dir
        .clone()
        .ok_or_else(|| format!("{} has no package folder to serve", ask.extension))?;
    let own = ask.extension.clone();
    let protocol = move |_id: wry::WebViewId<'_>, req: wry::http::Request<Vec<u8>>| {
        let path = req.uri().path().to_owned();
        match webview::serve(&own, &dir, &path) {
            Ok(asset) => {
                let mut res = Response::builder().status(StatusCode::OK);
                for (k, v) in &asset.headers {
                    res = res.header(*k, v.as_str());
                }
                res.body(Cow::Owned(asset.body))
                    .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..])))
            }
            Err(e) => Response::builder()
                .status(e.status())
                .header("content-type", "text/plain; charset=utf-8")
                .body(Cow::Owned(e.to_string().into_bytes()))
                .unwrap_or_else(|_| Response::new(Cow::Borrowed(&[][..]))),
        }
    };
    let (in_tx, inbox) = mpsc::channel::<String>();
    let (out_tx, outbox) = mpsc::channel::<Outbound>();
    let escape = Arc::new(AtomicBool::new(false));
    let back = Arc::clone(&escape);
    let bridge = Bridge::new(
        host,
        &ask.extension,
        Arc::new(move |m| {
            let _ = out_tx.send(m);
        }),
        move || back.store(true, Ordering::SeqCst),
    );
    let view = WebViewBuilder::new()
        .with_custom_protocol(webview::SCHEME.to_owned(), protocol)
        .with_initialization_script(webview::boot_script())
        .with_url(view_url(&ask.extension, &ask.src))
        .with_ipc_handler(move |req| {
            let _ = in_tx.send(req.into_body());
        })
        .with_navigation_handler(|url| may_navigate(&url))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_devtools(cfg!(debug_assertions))
        .with_bounds(rect)
        .build_as_child(&window)
        .map_err(|e| why_not(&e))?;
    let src = ask.dir.as_ref().map(|d| d.join(&ask.src));
    Ok(Native {
        changed: modified(src.as_deref()),
        src,
        checked: Instant::now(),
        view,
        bridge,
        inbox,
        outbox,
        bounds: None,
        visible: true,
        theme: String::new(),
        escape,
        parked: None,
    })
}

/// Drops parked views older than [`PARKED`].
fn reap() {
    VIEWS.with(|v| {
        v.borrow_mut()
            .retain(|_, n| n.parked.is_none_or(|t| t.elapsed() < PARKED));
    });
}

fn rect_of(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        position: LogicalPosition::new(x, y).into(),
        size: LogicalSize::new(w.max(1.0), h.max(1.0)).into(),
    }
}

/// The theme a view sees now: its scheme and its tokens as CSS.
fn theme_now() -> (String, String) {
    architect_theme::try_use_theme().map_or_else(
        || ("dark".to_owned(), webview::theme_css("dark", [])),
        |store| {
            let resolved = store.resolved();
            let resolved = resolved.read();
            let scheme = if resolved.scheme() == architect_theme::Scheme::Light {
                "light"
            } else {
                "dark"
            };
            let css = webview::theme_css(
                scheme,
                resolved.tokens().iter().map(|(k, v)| (k.as_str(), v.as_str())),
            );
            (scheme.to_owned(), css)
        },
    )
}

/// A `web` view in its pane.
#[component]
pub fn DeskWebPane(ask: WebAsk, host: HostRef) -> Element {
    let pane = use_pane();
    let key = format!("{}/{}#{}", ask.extension, ask.view, pane.id.0);
    let preview = pane.is_preview();
    let (scheme, css) = theme_now();
    let mut theme = use_signal(|| (scheme.clone(), css.clone()));
    if theme.peek().1 != css {
        theme.set((scheme, css));
    }
    let mut mounted = use_signal(|| None::<Rc<MountedData>>);
    let mut problem = use_signal(|| None::<String>);
    let live = try_use_context::<LiveDocument>();
    let window: Option<Arc<dyn OsWindow>> = try_use_context::<Arc<dyn OsWindow>>()
        .or_else(|| try_use_context::<MainWindow>().and_then(|m| m.window()));

    let drive_key = key.clone();
    let drive_ask = ask.clone();
    use_future(move || {
        let key = drive_key.clone();
        let ask = drive_ask.clone();
        let host = host.0.clone();
        let live = live.clone();
        let window = window.clone();
        async move {
            if preview {
                return;
            }
            loop {
                futures_timer::Delay::new(FRAME).await;
                pump();
                if problem.peek().is_some() {
                    continue;
                }
                let Some(m) = mounted.peek().clone() else {
                    continue;
                };
                let Ok(r) = m.get_client_rect().await else {
                    continue;
                };
                let (x, y, w, h) = (r.origin.x, r.origin.y, r.size.width, r.size.height);
                let sized = w >= 1.0 && h >= 1.0;
                // Nine points inside its rect: near the corners (clear of
                // a float's rounded ones), the edges' middles and its
                // centre.
                let inset = |len: f64| INSET.min(len / 2.0);
                let xs = [x + inset(w), x + w / 2.0, x + w - inset(w)];
                let ys = [y + inset(h), y + h / 2.0, y + h - inset(h)];
                let covered = live.as_ref().is_some_and(|doc| {
                    sized
                        && xs.iter().any(|px| {
                            ys.iter().any(|py| {
                                #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                                let (px, py) = (*px as f32, *py as f32);
                                doc.covered_at(px, py, HOLDER, &key) == Some(true)
                            })
                        })
                });
                let shown = Shown {
                    front: true,
                    sized,
                    overlays: u32::from(covered),
                };
                let mut work = Vec::new();
                let made = VIEWS.with(|views| {
                    let mut views = views.borrow_mut();
                    if !views.contains_key(&key) {
                        let Some(window) = window.as_ref() else {
                            return Err("This window has no OS window to put a web view in.".to_owned());
                        };
                        if !sized {
                            return Ok(());
                        }
                        let native = create(&host, &ask, window.as_ref(), rect_of(x, y, w, h))?;
                        views.insert(key.clone(), native);
                    }
                    let Some(n) = views.get_mut(&key) else {
                        return Ok(());
                    };
                    n.parked = None;
                    #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                    let bounds = Some((x as i64, y as i64, w as i64, h as i64));
                    if n.visible != shown.shown() {
                        n.visible = shown.shown();
                        let _ = n.view.set_visible(n.visible);
                        n.bridge.send(Outbound::Shown { shown: n.visible });
                        // Shown again, a child is put back where it was
                        // made (WebKitGTK's on X11): place it afresh.
                        n.bounds = None;
                    }
                    if n.bounds != bounds && sized && n.visible {
                        let _ = n.view.set_bounds(rect_of(x, y, w, h));
                        n.bounds = bounds;
                    }
                    let (scheme, css) = theme.peek().clone();
                    if n.theme != css {
                        n.bridge.send(Outbound::Theme { scheme, css: css.clone() });
                        n.theme = css;
                    }
                    while let Ok(text) = n.inbox.try_recv() {
                        if text.contains(r#""t":"ready""#) {
                            // A (re)loaded document: theme and shown again.
                            let (scheme, css) = theme.peek().clone();
                            n.bridge.send(Outbound::Theme { scheme, css });
                            n.bridge.send(Outbound::Shown { shown: n.visible });
                        }
                        if let Some(f) = n.bridge.receive(&text) {
                            work.push(f);
                        }
                    }
                    while let Ok(out) = n.outbox.try_recv() {
                        let text = serde_json::to_string(&out.to_json()).unwrap_or_default();
                        let _ = n.view.evaluate_script(&format!(
                            "window.__architectReceive&&window.__architectReceive({text})"
                        ));
                    }
                    if n.checked.elapsed() >= Duration::from_secs(1) {
                        n.checked = Instant::now();
                        let now = modified(n.src.as_deref());
                        if now != n.changed {
                            n.changed = now;
                            let _ = n.view.reload();
                        }
                    }
                    if n.escape.swap(false, Ordering::SeqCst) {
                        let _ = n.view.focus_parent();
                        if let Some(window) = window.as_ref() {
                            window.focus_window();
                        }
                    }
                    Ok(())
                });
                for f in work {
                    spawn(f);
                }
                if let Err(why) = made {
                    tracing::warn!(view = %key, %why, "a web view could not be shown");
                    problem.set(Some(why));
                }
            }
        }
    });

    let drop_key = key.clone();
    use_drop(move || {
        VIEWS.with(|views| {
            if let Some(n) = views.borrow_mut().get_mut(&drop_key) {
                let _ = n.view.set_visible(false);
                n.visible = false;
                n.parked = Some(Instant::now());
            }
        });
        reap();
        // Its loop is gone with it: hide it now (GTK acts on its next turn).
        pump();
    });

    if preview {
        return rsx! {
            div { style: "flex: 1; display: flex; align-items: center; justify-content: center; color: var(--muted-foreground); font-size: 13px;",
                "{ask.view}"
            }
        };
    }
    let message = problem();
    rsx! {
        div {
            "data-webview": "{key}",
            style: "flex: 1; min-height: 0; position: relative; display: flex; align-items: center; justify-content: center;",
            onmounted: move |e| mounted.set(Some(e.data())),
            if let Some(why) = message {
                div { style: "max-width: 420px; padding: 24px; font-size: 13px; line-height: 1.5; color: var(--muted-foreground); text-align: center;",
                    "{why}"
                }
            }
        }
    }
}
