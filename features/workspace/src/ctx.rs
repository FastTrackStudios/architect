//! [`ViewCtx`]: what every view kind is given when it mounts (feature
//! `ctx`; `docs/content/architecture/embedded-views.md`, "The contract").
//!
//! A host builds one per mounted view — a workspace pane, a canvas
//! overlay, an inline embed, a plugin window — and hands it to the kind's
//! provider. The built-in `dioxus` kind reads the same things through
//! hooks instead (`use_pane`, the app's Action List in context); every
//! other kind gets them here, so a GPU scene, a web view and an agent's
//! generated UI all see the pane the same way.
//!
//! No Dioxus and no clock of its own: the host passes its clock in
//! ([`FrameClock::new`]), so it builds on wasm and in a test.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect::action::{ActionCaller, ActionList};

use crate::PaneId;
use crate::kinds::{KindId, Placement, ViewHost};

/// A view's size in logical pixels and the display's scale factor.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Geometry {
    pub width: f32,
    pub height: f32,
    /// Physical pixels per logical one.
    pub scale: f32,
}

/// A document a view edits: the `CrdtDoc`, the container path in it
/// where the view's data lives, and the history its edits go through
/// (each one a named, undoable step on the host's undo).
#[derive(Clone)]
pub struct DocRef {
    pub doc: crdt::CrdtDoc,
    /// Where in `doc` (`"surfaces/panel"`, or `""` for the whole of it).
    pub path: String,
    pub history: Arc<Mutex<crdt::History>>,
}

impl std::fmt::Debug for DocRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DocRef")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

/// The app's theme as tokens: CSS custom properties by name, without the
/// leading `--` (`"accent"` → `"#4f7cff"`). A kind that does not draw in
/// a DOM (a GPU scene) reads the colours it needs from here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ThemeTokens(pub BTreeMap<String, String>);

impl ThemeTokens {
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }
}

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Primary,
    Secondary,
    Middle,
}

/// One input event, in the view's own logical coordinates: the set
/// remote views carry too (architect#176), so a kind sees the same events
/// in-process and across a process boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum ViewInput {
    PointerDown {
        x: f32,
        y: f32,
        button: Button,
    },
    PointerMove {
        x: f32,
        y: f32,
    },
    PointerUp {
        x: f32,
        y: f32,
        button: Button,
    },
    /// Scrolled by `dx, dy` logical pixels.
    Wheel {
        dx: f32,
        dy: f32,
    },
    /// A finger, by its id: `phase` is `"start"`, `"move"`, `"end"` or
    /// `"cancel"`.
    Touch {
        id: u64,
        x: f32,
        y: f32,
        phase: &'static str,
    },
    /// A key by its `KeyboardEvent.key` name, with the held modifiers
    /// (`"Mod"`, `"Shift"`, `"Alt"`, `"Ctrl"`).
    Key {
        key: String,
        down: bool,
        modifiers: Vec<&'static str>,
    },
    /// Composed text: `preedit` while composing, then `commit`.
    Ime {
        preedit: String,
        commit: Option<String>,
    },
    /// The view got (`true`) or lost the keyboard focus.
    Focus(bool),
}

/// Where a host sends a view's input: the host pushes, the kind drains
/// (once a frame, for one that draws per frame). Cheap to clone.
#[derive(Debug, Clone, Default)]
pub struct InputSink(Arc<Mutex<VecDeque<ViewInput>>>);

impl InputSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The host: one more event.
    pub fn push(&self, event: ViewInput) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(event);
    }

    /// The kind: every event since the last drain, oldest first.
    #[must_use]
    pub fn drain(&self) -> Vec<ViewInput> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .drain(..)
            .collect()
    }
}

/// A clock the host owns: the time now, as a duration since any fixed
/// point (`web_time::Instant` on a desk, `performance.now()` in a
/// browser, a virtual clock in a film or a test).
pub type Now = Arc<dyn Fn() -> Duration + Send + Sync>;

/// The frame clock: when a frame began and how long a kind may spend in
/// it. A kind that misses its budget skips work (a lower-detail scene),
/// never the host's frame.
#[derive(Clone)]
pub struct FrameClock {
    now: Now,
    budget: Duration,
}

impl std::fmt::Debug for FrameClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrameClock")
            .field("budget", &self.budget)
            .finish_non_exhaustive()
    }
}

impl FrameClock {
    /// A clock reading `now`, giving each frame `budget`.
    pub fn new(budget: Duration, now: impl Fn() -> Duration + Send + Sync + 'static) -> Self {
        Self {
            now: Arc::new(now),
            budget,
        }
    }

    /// How long a frame may take.
    #[must_use]
    pub const fn budget(&self) -> Duration {
        self.budget
    }

    /// The time now.
    #[must_use]
    pub fn now(&self) -> Duration {
        (self.now)()
    }

    /// A frame starting now.
    #[must_use]
    pub fn frame(&self) -> Frame {
        Frame {
            began: self.now(),
            clock: self.clone(),
        }
    }
}

/// One frame of a [`FrameClock`].
#[derive(Debug, Clone)]
pub struct Frame {
    began: Duration,
    clock: FrameClock,
}

impl Frame {
    /// What is left of its budget (zero once it is spent).
    #[must_use]
    pub fn left(&self) -> Duration {
        let spent = self.clock.now().saturating_sub(self.began);
        self.clock.budget.saturating_sub(spent)
    }

    #[must_use]
    pub fn over(&self) -> bool {
        self.left().is_zero()
    }
}

/// Typed handles a host adds for kinds that want them: the canvas host's
/// publisher for the `Spaces` a view draws in (`architect-canvas`), a
/// frame engine. Looked up by type.
#[derive(Clone, Default)]
pub struct Extras(BTreeMap<TypeId, Arc<dyn Any + Send + Sync>>);

impl std::fmt::Debug for Extras {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extras")
            .field("len", &self.0.len())
            .finish()
    }
}

impl Extras {
    #[must_use]
    pub fn with<T: Any + Send + Sync>(mut self, value: T) -> Self {
        self.0.insert(TypeId::of::<T>(), Arc::new(value));
        self
    }

    #[must_use]
    pub fn get<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.0.get(&TypeId::of::<T>())?.downcast_ref()
    }
}

/// What every view kind is given when it mounts.
#[derive(Clone, Debug)]
pub struct ViewCtx {
    /// The pane it is drawn in (an overlay's or embed's own id otherwise).
    pub pane: PaneId,
    /// The kind it is drawn by.
    pub kind: KindId,
    pub host: ViewHost,
    pub placement: Placement,
    pub geometry: Geometry,
    /// The host's Action List: the only way a view changes anything
    /// outside itself.
    pub actions: ActionList,
    /// Who its runs are made as: the person at the host, or an agent for
    /// its generated UI (confirmations and the authorizer see this).
    pub caller: ActionCaller,
    /// The document it edits, if any.
    pub doc: Option<DocRef>,
    pub theme: ThemeTokens,
    pub input: InputSink,
    pub clock: FrameClock,
    pub extras: Extras,
}

impl ViewCtx {
    /// A context for `pane` drawn by `kind` in `host` as `placement`, on
    /// `actions` as `caller`, at `clock`: no document, no theme, no
    /// extras, a zero size until the host measures it.
    #[must_use]
    pub fn new(
        pane: PaneId,
        kind: KindId,
        (host, placement): (ViewHost, Placement),
        (actions, caller): (ActionList, ActionCaller),
        clock: FrameClock,
    ) -> Self {
        Self {
            pane,
            kind,
            host,
            placement,
            geometry: Geometry::default(),
            actions,
            caller,
            doc: None,
            theme: ThemeTokens::default(),
            input: InputSink::new(),
            clock,
            extras: Extras::default(),
        }
    }

    #[must_use]
    pub fn with_doc(mut self, doc: DocRef) -> Self {
        self.doc = Some(doc);
        self
    }

    #[must_use]
    pub fn with_theme(mut self, theme: ThemeTokens) -> Self {
        self.theme = theme;
        self
    }

    #[must_use]
    pub const fn with_geometry(mut self, geometry: Geometry) -> Self {
        self.geometry = geometry;
        self
    }

    #[must_use]
    pub fn with_extras(mut self, extras: Extras) -> Self {
        self.extras = extras;
        self
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    #[test]
    fn a_frame_spends_its_budget_on_the_hosts_clock() {
        let ms = Arc::new(AtomicU64::new(0));
        let read = Arc::clone(&ms);
        let clock = FrameClock::new(Duration::from_millis(4), move || {
            Duration::from_millis(read.load(Ordering::Relaxed))
        });
        let frame = clock.frame();
        assert_eq!(frame.left(), Duration::from_millis(4));
        ms.store(3, Ordering::Relaxed);
        assert!(!frame.over());
        ms.store(9, Ordering::Relaxed);
        assert!(frame.over());
    }

    #[test]
    fn input_drains_in_order() {
        let sink = InputSink::new();
        sink.push(ViewInput::Focus(true));
        sink.push(ViewInput::Wheel { dx: 0.0, dy: 3.0 });
        assert_eq!(sink.drain().len(), 2);
        assert!(sink.drain().is_empty());
    }

    #[test]
    fn a_ctx_carries_the_hosts_list_and_extras() {
        #[derive(Debug, PartialEq)]
        struct Engine(u8);
        let ctx = ViewCtx::new(
            PaneId::new("p1"),
            KindId::dioxus(),
            (ViewHost::Blitz, Placement::Pane),
            (ActionList::new(), ActionCaller::app()),
            FrameClock::new(Duration::from_millis(8), || Duration::ZERO),
        )
        .with_extras(Extras::default().with(Engine(7)))
        .with_theme(ThemeTokens::default().with("accent", "#4f7cff"));
        assert_eq!(ctx.extras.get::<Engine>(), Some(&Engine(7)));
        assert_eq!(ctx.theme.get("accent"), Some("#4f7cff"));
        assert!(ctx.doc.is_none());
        assert!(ctx.actions.infos().is_empty());
    }
}
