//! The app's tab kinds in the host: [`use_tab_kinds`] puts its
//! [`TabKindsUi`] (the kinds, and the renderers some of them bring) in
//! context, where the tab bar reads it, and `TAB_KINDS` on the app's
//! Action List.
//!
//! A kind with no renderer draws its [`TabFace`] the bar's way (an icon in
//! its colour, its title, a wide kind's subtitle under it, its badge). A
//! renderer draws what goes inside the tab instead; the bar keeps the
//! tab's frame (its press, its menu, its drag, its ×).

use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};

use architect_shell::ShellTheme;
use dioxus::prelude::*;

use crate::tab_kind::{TabFace, TabKinds};

/// What a kind's renderer is given for one tab.
#[derive(Clone, PartialEq)]
pub struct TabRenderCtx {
    pub face: TabFace,
    /// It is the one on show (a scope tab active, a pane current).
    pub on: bool,
    /// The room it has, in points.
    pub height: f32,
    pub theme: ShellTheme,
}

/// A kind's own drawing of what is inside its tabs.
pub type TabRender = Rc<dyn Fn(TabRenderCtx) -> Element>;

/// The app's tab kinds and their renderers.
#[derive(Clone, Default)]
pub struct TabKindsUi {
    pub kinds: TabKinds,
    renderers: Vec<(String, TabRender)>,
}

/// The same kinds, and a renderer for the same ones: an app that makes
/// its renderers afresh each render (a new `Rc`) does not redraw every bar.
impl PartialEq for TabKindsUi {
    fn eq(&self, other: &Self) -> bool {
        self.kinds == other.kinds
            && self.renderers.len() == other.renderers.len()
            && self
                .renderers
                .iter()
                .zip(&other.renderers)
                .all(|((a, _), (b, _))| a == b)
    }
}

impl TabKindsUi {
    #[must_use]
    pub fn new(kinds: TabKinds) -> Self {
        Self {
            kinds,
            renderers: Vec::new(),
        }
    }

    /// Kind `kind`'s tabs draw with `render`.
    #[must_use]
    pub fn renderer(mut self, kind: impl Into<String>, render: TabRender) -> Self {
        let kind = kind.into();
        self.renderers.retain(|(k, _)| *k != kind);
        self.renderers.push((kind, render));
        self
    }

    /// Kind `kind`'s renderer, when it brought one.
    #[must_use]
    pub fn render_for(&self, kind: &str) -> Option<TabRender> {
        self.renderers
            .iter()
            .find(|(k, _)| k == kind)
            .map(|(_, r)| r.clone())
    }
}

/// The tab kinds in context: a signal, so a kind an extension adds as it
/// loads redraws the bars.
#[derive(Clone, Copy, PartialEq)]
struct TabKindsCtx(Signal<TabKindsUi>);

/// Puts `kinds` in context for the tab bars under it, and `TAB_KINDS` on
/// `actions` while mounted. Call it beside the app's `WorkspaceView`
/// (before it, in the same component or above).
pub fn use_tab_kinds(kinds: TabKindsUi, actions: Option<super::WorkspaceActions>) {
    let ctx = use_context_provider(|| TabKindsCtx(Signal::new(kinds.clone())));
    let mirror: Arc<Mutex<TabKinds>> = use_hook(|| Arc::new(Mutex::new(kinds.kinds.clone())));
    if *ctx.0.peek() != kinds {
        *mirror.lock().unwrap_or_else(PoisonError::into_inner) = kinds.kinds.clone();
        let mut s = ctx.0;
        s.set(kinds);
    }
    let registered = use_hook(move || {
        let list = actions?.0;
        let read = mirror;
        let kinds: Arc<dyn Fn() -> TabKinds + Send + Sync> =
            Arc::new(move || read.lock().unwrap_or_else(PoisonError::into_inner).clone());
        crate::control::deck::register_tab_kinds(&list, kinds).ok()?;
        Some(list)
    });
    use_drop(move || {
        if let Some(list) = registered {
            list.unregister(crate::control::deck::ids::TAB_KINDS);
        }
    });
}

/// The tab kinds under [`use_tab_kinds`], if the app gave any.
#[must_use]
pub fn try_use_tab_kinds() -> Option<TabKindsUi> {
    try_use_context::<TabKindsCtx>().map(|c| c.0.read().clone())
}
