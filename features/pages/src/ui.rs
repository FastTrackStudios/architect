//! A page in a pane (feature `ui`).
//!
//! [`PageView`] mounts a Blitz sub-document in its element — the same
//! `RemoteDocument` a process extension's view fills, with the host's
//! live theme CSS, events, focus and IME — and applies the page's frames
//! to it as they come from the page's thread. The window's thread only
//! applies frames: it never runs the page.
//!
//! Each frame is timed from the event that asked for it into the view's
//! `architect_workspace::ViewMeter` (`<extension>/<view>` in the
//! `Meters` in context; `EXT_VIEWS` reads it), and while an event goes
//! unanswered past [`FRAME_BUDGET`] the pane keeps its last frame with
//! the shell's busy mark.
//!
//! [`use_view_pages`] provides `architect_extensions::interface::ViewPages`:
//! every `ExtensionView` with a page — an override in the app's folder,
//! its manifest's `page`, a built-in's page file — is drawn by a
//! [`PageView`], and one whose override is saved or deleted switches
//! within a moment (the folder is looked at every [`OVERRIDES_EVERY`]).

// Dioxus is single-threaded: its tasks are not `Send`.
#![allow(clippy::future_not_send)]

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use architect_extensions::interface::{PageAsk, ViewPages};
use architect_extensions::remote::RemoteDocument;
use architect_extensions::wire::{Frame, ViewEvent};
use architect_scripting::luau::Gate;
use architect_workspace::{FRAME_BUDGET, Meters, ViewMeter};
use blitz_dom::{BaseDocument, NodeId};
use dioxus::prelude::*;
use dioxus_native_dom::NodeHandle;
use futures::channel::mpsc;
use futures::{FutureExt as _, StreamExt as _};

use crate::files::{Origin, PageFile};
use crate::runner::Pages;

/// How often the app's `pages/` folder is looked at for an override that
/// came or went.
pub const OVERRIDES_EVERY: Duration = Duration::from_millis(300);

/// A page to mount: which file, as whose view, reaching what.
#[derive(Clone)]
pub struct Mount {
    pub pages: Pages,
    pub page: PageFile,
    pub extension: String,
    pub view: String,
    pub gate: Option<Gate>,
}

impl PartialEq for Mount {
    fn eq(&self, other: &Self) -> bool {
        self.page == other.page
            && self.extension == other.extension
            && self.view == other.view
            && self.pages.same(&other.pages)
    }
}

impl std::fmt::Debug for Mount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mount")
            .field("page", &self.page.path)
            .field("extension", &self.extension)
            .field("view", &self.view)
            .finish_non_exhaustive()
    }
}

/// Provides `ViewPages` for `pages`: extension views with a page file are
/// drawn from it. Call it once, near the app's root.
#[must_use]
pub fn use_view_pages(pages: Pages) -> ViewPages {
    let mut revision = use_signal(|| 0_u64);
    let watched = pages.clone();
    use_hook(move || {
        spawn(async move {
            let mut seen = watched.app_dir().map(crate::files::overrides_print);
            loop {
                futures_timer::Delay::new(OVERRIDES_EVERY).await;
                let now = watched.app_dir().map(crate::files::overrides_print);
                if now != seen {
                    seen = now;
                    let next = revision.peek().wrapping_add(1);
                    revision.set(next);
                }
            }
        });
    });
    let provided = use_hook(move || {
        ViewPages(Rc::new(move |ask: &PageAsk| {
            // Read: the view draws again when an override comes or goes.
            let _ = revision();
            let own = ask
                .page
                .as_ref()
                .zip(ask.dir.as_ref())
                .and_then(|(page, dir)| PageFile::new(dir.join(page), Origin::Extension));
            let page = pages.resolve(&ask.extension, &ask.view, own)?;
            let mount = Mount {
                pages: pages.clone(),
                page,
                extension: ask.extension.clone(),
                view: ask.view.clone(),
                gate: Some(Arc::clone(&ask.gate)),
            };
            let key = mount.page.path.display().to_string();
            Some(rsx! { PageView { key: "{key}", mount } })
        }))
    });
    use_context_provider(|| provided.clone());
    provided
}

/// Runs `f` on the host's document, when it is free.
async fn with_doc<R>(node: &NodeHandle, f: impl FnOnce(&mut BaseDocument, NodeId) -> R) -> R {
    loop {
        if node.try_doc().is_some() {
            let mut doc = node.doc_mut();
            return f(&mut doc, node.node_id());
        }
        futures_timer::Delay::new(Duration::from_millis(8)).await;
    }
}

fn remote(doc: &mut BaseDocument, node: NodeId) -> Option<&mut RemoteDocument> {
    let sub: &mut dyn std::any::Any = doc.subdoc_mut(node)?;
    sub.downcast_mut::<RemoteDocument>()
}

/// A page, filling its parent: the page's frames in a sub-document of its
/// own, its pane's events back to its handlers.
#[component]
pub fn PageView(mount: Mount) -> Element {
    #[cfg(feature = "a2ui")]
    if mount.page.kind == crate::files::PageKind::A2ui {
        return rsx! { crate::a2ui::A2uiPage { mount } };
    }
    let busy = use_signal(|| false);
    let why = use_signal(|| None::<String>);
    let meters = try_use_context::<Meters>();
    let meter = use_hook(|| {
        let name = format!("{}/{}", mount.extension, mount.view);
        meters.map_or_else(
            || ViewMeter::new(&name, FRAME_BUDGET),
            |m| m.meter(&name, FRAME_BUDGET),
        )
    });
    {
        let meter = meter.clone();
        use_drop(move || meter.set_busy(false));
    }
    let theme = architect_theme::try_use_root_css();
    let mut node_at = use_signal(|| None::<NodeHandle>);
    use_effect(move || {
        let Some(css) = theme.map(|t| t()) else {
            return;
        };
        let Some(node) = node_at.peek().clone() else {
            return;
        };
        spawn(async move {
            with_doc(&node, |doc, at| {
                if let Some(r) = remote(doc, at) {
                    r.set_theme_css(&css);
                }
            })
            .await;
        });
    });
    let mut started = use_signal(|| false);
    let shown = why();
    rsx! {
        div { class: "page-view", style: "position: relative; width: 100%; height: 100%; overflow: auto;",
            div {
                style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0;",
                onmounted: move |e: MountedEvent| {
                    if !started() && let Some(node) = e.data().downcast::<NodeHandle>() {
                        started.set(true);
                        node_at.set(Some(node.clone()));
                        spawn(follow(node.clone(), mount.clone(), theme, meter.clone(), busy, why));
                    }
                },
            }
            if busy() && shown.is_none() {
                architect_shell::BusyBadge {}
            }
            if let Some(why) = shown {
                div { class: "page-view-why",
                    style: "position: absolute; top: 0; left: 0; right: 0; bottom: 0; display: flex; \
                            align-items: center; justify-content: center; padding: 24px; font-size: 13px; \
                            color: var(--muted-foreground); text-align: center;",
                    "{why}"
                }
            }
        }
    }
}

/// Opens the page and follows it: its frames into the document, timed,
/// until it stops.
async fn follow(
    node: NodeHandle,
    mount: Mount,
    theme: Option<Signal<String>>,
    meter: ViewMeter,
    mut busy: Signal<bool>,
    mut why: Signal<Option<String>>,
) {
    let mut session = match mount.pages.open(
        &mount.page,
        &mount.extension,
        &mount.view,
        mount.gate.clone(),
    ) {
        Ok(s) => s,
        Err(e) => {
            why.set(Some(e));
            return;
        }
    };
    let (events, mut from_doc) = mpsc::unbounded::<ViewEvent>();
    with_doc(&node, |doc, at| {
        let mut r = RemoteDocument::new(events);
        if let Some(css) = theme {
            r.set_theme_css(&css.peek());
        }
        doc.set_sub_document(at, Box::new(r));
    })
    .await;
    let asked: Rc<RefCell<VecDeque<Instant>>> = Rc::default();
    let (asking, mut was_asked) = mpsc::unbounded::<()>();
    let forwarding = {
        let (sender, asked) = (session.sender(), Rc::clone(&asked));
        spawn(async move {
            while let Some(event) = from_doc.next().await {
                asked.borrow_mut().push_back(Instant::now());
                let _ = asking.unbounded_send(());
                if !sender.event(event) {
                    return;
                }
            }
        })
    };
    while let Some(frame) =
        next_frame(&mut session.frames, &asked, &mut was_asked, &meter, busy).await
    {
        if let Some(at) = asked.borrow_mut().pop_front() {
            meter.record(at.elapsed());
        } else {
            meter.set_busy(false);
        }
        if *busy.peek() {
            busy.set(false);
        }
        let applied = with_doc(&node, |doc, at| {
            remote(doc, at).map_or_else(
                || Err("the page was unmounted".to_owned()),
                |r| r.apply(frame),
            )
        })
        .await;
        if let Err(e) = applied {
            why.set(Some(format!("{}'s page failed: {e}", mount.extension)));
            break;
        }
    }
    forwarding.cancel();
    meter.set_busy(false);
    drop(session);
    with_doc(&node, BaseDocument::remove_sub_document).await;
}

/// The next frame; meanwhile, once the oldest unanswered event is past
/// the budget, the view is busy.
async fn next_frame(
    frames: &mut mpsc::UnboundedReceiver<Frame>,
    asked: &Rc<RefCell<VecDeque<Instant>>>,
    was_asked: &mut mpsc::UnboundedReceiver<()>,
    meter: &ViewMeter,
    mut busy: Signal<bool>,
) -> Option<Frame> {
    loop {
        let oldest = asked.borrow().front().copied();
        let budget = meter.budget();
        let overdue = async {
            match oldest {
                Some(at) if !*busy.peek() => {
                    futures_timer::Delay::new(budget.saturating_sub(at.elapsed())).await;
                }
                _ => std::future::pending().await,
            }
        };
        futures::select! {
            frame = frames.next() => return frame,
            again = was_asked.next() => {
                if again.is_none() {
                    return frames.next().await;
                }
            }
            () = Box::pin(overdue).fuse() => {
                meter.set_busy(true);
                busy.set(true);
            }
        }
    }
}
