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
    // Parked views are reaped on a clock of their own, not only when
    // another pane goes.
    use_future(|| async {
        loop {
            futures_timer::Delay::new(REAP).await;
            reap();
            pump();
        }
    });
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
    /// The OS window it is a child of: a pane moved to another (a pop-out)
    /// gets a view of its own there.
    window: dioxus_native::winit::window::WindowId,
    /// When it last gave the focus back (an Escape), to debounce.
    escaped_at: Option<Instant>,
    /// Frames its pane has been uncovered in a row (it shows after two).
    clear_frames: u8,
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

/// The extension host and the path a view's URL names: `architect-ext://<id>/<path>`,
/// or `WebView2`'s `http://architect-ext.<id>/<path>`. `None` for anything else.
pub(crate) fn origin_of(url: &str) -> Option<(&str, &str)> {
    let rest = url
        .strip_prefix(webview::SCHEME)
        .and_then(|r| r.strip_prefix("://"))
        .or_else(|| {
            url.strip_prefix("http://")
                .and_then(|r| r.strip_prefix(webview::SCHEME))
                .and_then(|r| r.strip_prefix('.'))
        })?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.split(['?', '#']).next().unwrap_or(host);
    Some((host, path))
}

/// Whether a view of extension `own` may navigate to `url`: only within its
/// own package's origin (or a blank page).
pub(crate) fn may_navigate(own: &str, url: &str) -> bool {
    url == "about:blank" || origin_of(url).is_some_and(|(host, _)| host == own)
}

/// Whether `id` can be a view's origin: a lowercase host name (labels of
/// `a-z 0-9 -`, none empty or starting or ending with `-`).
pub(crate) fn hostname_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 253
        && id.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        })
}

/// What the protocol answers a view of `own` (package `dir`) for `uri`: its
/// own package's file, or a refusal for another origin.
pub(crate) fn answer(
    own: &str,
    dir: &std::path::Path,
    uri: &str,
) -> Result<webview::Asset, webview::AssetError> {
    let Some((host, path)) = origin_of(uri) else {
        return Err(webview::AssetError::Outside(uri.to_owned()));
    };
    // The full URL, so `serve` refuses another extension's origin.
    webview::serve(own, dir, &webview::url(host, path))
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
fn create(host: &Host, ask: &WebAsk, window: &dyn OsWindow, rect: Rect) -> Result<Native, String> {
    pump();
    let dir = ask
        .dir
        .clone()
        .ok_or_else(|| format!("{} has no package folder to serve", ask.extension))?;
    if !hostname_ok(&ask.extension) {
        return Err(format!(
            "{} cannot be a web view's origin: an extension with web views needs an id that is a host name (a-z, 0-9, - and .)",
            ask.extension
        ));
    }
    let own = ask.extension.clone();
    let protocol = move |_id: wry::WebViewId<'_>, req: wry::http::Request<Vec<u8>>| {
        let uri = req.uri().to_string();
        match answer(&own, &dir, &uri) {
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
    // Bounded both ways: a view past them loses messages, never the UI
    // thread's memory or time.
    let (in_tx, inbox) = mpsc::sync_channel::<String>(webview::MAX_QUEUED);
    let (out_tx, outbox) = mpsc::sync_channel::<Outbound>(webview::MAX_QUEUED);
    let escape = Arc::new(AtomicBool::new(false));
    let back = Arc::clone(&escape);
    let bridge = Bridge::new(
        host,
        &ask.extension,
        Arc::new(move |m| {
            let _ = out_tx.try_send(m);
        }),
        move || back.store(true, Ordering::SeqCst),
    );
    let nav_own = ask.extension.clone();
    let view = WebViewBuilder::new()
        // Its own ephemeral storage: no view shares cookies, local storage
        // or IndexedDB with another's.
        .with_incognito(true)
        .with_custom_protocol(webview::SCHEME.to_owned(), protocol)
        .with_initialization_script(webview::boot_script())
        .with_url(view_url(&ask.extension, &ask.src))
        .with_ipc_handler(move |req| {
            let body = req.into_body();
            if body.len() <= webview::MAX_MESSAGE {
                let _ = in_tx.try_send(body);
            }
        })
        .with_navigation_handler(move |url| may_navigate(&nav_own, &url))
        .with_new_window_req_handler(|_, _| NewWindowResponse::Deny)
        .with_devtools(cfg!(debug_assertions))
        .with_bounds(rect)
        .build_as_child(&window)
        .map_err(|e| why_not(&e))?;
    let src = ask.dir.as_ref().map(|d| d.join(&ask.src));
    Ok(Native {
        window: window.id(),
        escaped_at: None,
        clear_frames: 0,
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

/// Drops parked views older than [`PARKED`], and empties the others'
/// queues: a parked view's messages are not acted on.
fn reap() {
    VIEWS.with(|v| {
        let mut v = v.borrow_mut();
        v.retain(|_, n| n.parked.is_none_or(|t| t.elapsed() < PARKED));
        for n in v.values_mut().filter(|n| n.parked.is_some()) {
            while n.inbox.try_recv().is_ok() {}
            while n.outbox.try_recv().is_ok() {}
        }
    });
}

/// How often parked views are reaped and drained, with or without a pane.
pub const REAP: Duration = Duration::from_secs(5);

/// Where a pane is probed for anything of the app's over it: a 5×5 grid,
/// [`INSET`] in from its edges.
fn probes(left: f64, top: f64, width: f64, height: f64) -> Vec<(f64, f64)> {
    let inset = |len: f64| INSET.min(len / 2.0);
    let along = |start: f64, len: f64| -> Vec<f64> {
        let (first, last) = (start + inset(len), start + len - inset(len));
        (0..5)
            .map(|i| first + (last - first) * f64::from(i) / 4.0)
            .collect()
    };
    let rows = along(top, height);
    along(left, width)
        .into_iter()
        .flat_map(|px| rows.iter().map(move |py| (px, *py)))
        .collect()
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
                resolved
                    .tokens()
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.as_str())),
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
                // A grid of points inside its rect (clear of a float's
                // rounded corners): anything of the app's over any of them
                // (a menu, a tooltip) covers it, and so does a document too
                // busy to answer.
                let covered = live.as_ref().is_some_and(|doc| {
                    sized
                        && probes(x, y, w, h).into_iter().any(|(px, py)| {
                            #[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
                            let (px, py) = (px as f32, py as f32);
                            doc.covered_at(px, py, HOLDER, &key) != Some(false)
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
                            return Err(
                                "This window has no OS window to put a web view in.".to_owned()
                            );
                        };
                        if !sized {
                            return Ok(());
                        }
                        let native = create(&host, &ask, window.as_ref(), rect_of(x, y, w, h))?;
                        views.insert(key.clone(), native);
                    }
                    // Its pane moved to another window (a pop-out): a child
                    // of that one now.
                    let here = window.as_ref().map(|w| w.id());
                    if views.get(&key).is_some_and(|n| Some(n.window) != here) {
                        views.remove(&key);
                        let Some(window) = window.as_ref() else {
                            return Ok(());
                        };
                        let native = create(&host, &ask, window.as_ref(), rect_of(x, y, w, h))?;
                        views.insert(key.clone(), native);
                    }
                    let Some(n) = views.get_mut(&key) else {
                        return Ok(());
                    };
                    if n.parked.take().is_some() {
                        n.bridge.set_paused(false);
                    }
                    // Hidden at once when covered; shown after two clear
                    // frames, so a busy frame does not make it flicker.
                    n.clear_frames = if shown.shown() {
                        n.clear_frames.saturating_add(1)
                    } else {
                        0
                    };
                    let shown = Shown {
                        overlays: u32::from(n.clear_frames < 2),
                        ..shown
                    };
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
                        n.bridge.send(Outbound::Theme {
                            scheme,
                            css: css.clone(),
                        });
                        n.theme = css;
                    }
                    for _ in 0..webview::MAX_PER_FRAME {
                        let Ok(text) = n.inbox.try_recv() else { break };
                        if matches!(webview::parse(&text), Ok(webview::Inbound::Ready)) {
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
                    // An Escape the page did not take: the focus back to the
                    // app's document, only while this window has the
                    // keyboard, at most once a quarter second, and never by
                    // raising the window.
                    if n.escape.swap(false, Ordering::SeqCst)
                        && window.as_ref().is_some_and(|w| w.has_focus())
                        && n.escaped_at
                            .is_none_or(|t| t.elapsed() >= Duration::from_millis(250))
                    {
                        n.escaped_at = Some(Instant::now());
                        let _ = n.view.focus_parent();
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
                // Nothing it hears while away is passed on or kept.
                n.bridge.set_paused(true);
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

#[cfg(test)]
mod tests {
    use super::{answer, hostname_ok, may_navigate, origin_of, probes};

    fn package() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "mine").unwrap();
        dir
    }

    #[test]
    fn a_view_is_served_its_own_origin_only() {
        let dir = package();
        let mine = answer(
            "demo.board",
            dir.path(),
            "architect-ext://demo.board/index.html",
        )
        .unwrap();
        assert_eq!(mine.body, b"mine");
        // WebView2's form of the same origin.
        assert!(
            answer(
                "demo.board",
                dir.path(),
                "http://architect-ext.demo.board/index.html"
            )
            .is_ok()
        );
        // Its own files at another extension's origin: refused.
        for uri in [
            "architect-ext://other.ext/index.html",
            "http://architect-ext.other.ext/index.html",
            "https://evil.example/index.html",
        ] {
            assert!(answer("demo.board", dir.path(), uri).is_err(), "{uri}");
        }
    }

    #[test]
    fn a_view_navigates_within_its_own_origin_only() {
        assert!(may_navigate(
            "demo.board",
            "architect-ext://demo.board/page.html#x"
        ));
        assert!(may_navigate(
            "demo.board",
            "http://architect-ext.demo.board/"
        ));
        assert!(may_navigate("demo.board", "about:blank"));
        for url in [
            "architect-ext://other.ext/index.html",
            "http://architect-ext.evil.com/",
            "http://architect-ext.demo.board.evil.com/",
            "https://example.com/",
            "architect-ext://demo.boarder/",
        ] {
            assert!(!may_navigate("demo.board", url), "{url}");
        }
        assert_eq!(
            origin_of("architect-ext://a.b/c/d.js?q=1"),
            Some(("a.b", "c/d.js?q=1"))
        );
    }

    #[test]
    fn an_origin_is_a_host_name() {
        assert!(hostname_ok("demo.webview-board"));
        for bad in ["", "demo_board", "Demo.board", "-x.y", "x..y", "a/b"] {
            assert!(!hostname_ok(bad), "{bad}");
        }
    }

    #[test]
    fn a_pane_is_probed_on_a_grid() {
        let p = probes(0.0, 0.0, 400.0, 200.0);
        assert_eq!(p.len(), 25);
        assert!(
            p.iter()
                .all(|(x, y)| (24.0..=376.0).contains(x) && (24.0..=176.0).contains(y))
        );
    }
}
