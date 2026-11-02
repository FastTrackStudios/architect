//! The Blitz host of the paint kind (feature `paint-ui`): a
//! [`PaintWidget`] is a Blitz custom widget whose picture is a
//! [`PaintView`]'s scene, and [`PaintPane`] mounts one in a workspace pane
//! with the pane's [`ViewCtx`].
//!
//! Blitz hands the widget its events in the element's own coordinates
//! and asks it for a scene at the element's size in physical pixels; the
//! scene is drawn by the window's renderer (Vello on the GPU), in the
//! page or as a layer of its own.

use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use anyrender::RenderContext;
use architect::action::{ActionCaller, ActionList};
use architect_shell::ShellTheme;
use blitz_dom::node::{ComputedStyles, Widget};
use blitz_traits::events::{
    BlitzImeEvent, BlitzPointerEvent, BlitzPointerId, BlitzWheelDelta, KeyState, Modifiers,
    MouseEventButton, UiEvent,
};
use blitz_traits::shell::ShellProvider;
use dioxus::prelude::*;
use dioxus_native_dom::CustomWidgetAttr;

use super::{FrameStats, PaintView, Painter, Scene};
use crate::ctx::{Button, Extras, FrameClock, Geometry, ThemeTokens, ViewCtx, ViewInput};
use crate::kinds::{KindId, Placement, ViewHost};

/// Logical pixels a wheel's line is.
const LINE: f64 = 20.0;

/// A [`Painter`] as a Blitz custom widget.
pub struct PaintWidget {
    painter: Painter,
    wake: PaintWake,
}

impl std::fmt::Debug for PaintWidget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("PaintWidget").field(&self.painter).finish()
    }
}

impl PaintWidget {
    #[must_use]
    pub fn new(painter: Painter) -> Self {
        Self {
            painter,
            wake: PaintWake::new(),
        }
    }

    fn with_wake(painter: Painter, wake: PaintWake) -> Self {
        Self { painter, wake }
    }

    fn frame(&mut self, geometry: Geometry) -> Scene {
        self.wake.0.dirty.swap(false, Ordering::AcqRel);
        self.sync_theme();
        self.painter.frame(geometry)
    }
}

impl Widget for PaintWidget {
    fn handle_event(&mut self, event: &UiEvent) {
        self.sync_theme();
        if let Some(input) = view_input(event) {
            self.painter.ctx().input.push(input);
        }
        self.painter.pump();
    }

    fn needs_redraw(&self) -> bool {
        self.wake.0.dirty.load(Ordering::Acquire) || self.painter.wants_frame()
    }

    fn requires_redraw(&self) -> bool {
        self.painter.animating()
    }

    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::as_conversions
    )]
    fn paint(
        &mut self,
        _: &mut dyn RenderContext,
        _: &ComputedStyles,
        width: u32,
        height: u32,
        scale: f64,
    ) -> Scene {
        let scale = if scale > 0.0 { scale } else { 1.0 };
        self.frame(Geometry {
            width: (f64::from(width) / scale) as f32,
            height: (f64::from(height) / scale) as f32,
            scale: scale as f32,
        })
    }
}

/// A cloneable, thread-safe request for a mounted pane to draw again.
/// Requests before mounting are retained. Use one handle per mounted pane.
#[derive(Clone, Default)]
pub struct PaintWake(Arc<WakeState>);

#[derive(Default)]
struct WakeState {
    dirty: AtomicBool,
    window: Mutex<Option<Arc<dyn ShellProvider>>>,
    theme: Mutex<Option<ThemeTokens>>,
}

impl PaintWidget {
    fn sync_theme(&mut self) {
        let theme = self
            .wake
            .0
            .theme
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(theme) = theme {
            self.painter.set_theme(theme);
        }
    }
}

impl PaintWake {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Mark the pane dirty and wake its window, including from a worker thread.
    pub fn wake(&self) {
        self.0.dirty.store(true, Ordering::Release);
        let window = self
            .0
            .window
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        if let Some(window) = window {
            window.request_redraw();
        }
    }

    fn attach(&self, window: Option<Arc<dyn ShellProvider>>) {
        *self.0.window.lock().unwrap_or_else(PoisonError::into_inner) = window;
        if self.0.dirty.load(Ordering::Acquire) {
            self.wake();
        }
    }

    fn set_theme(&self, theme: ThemeTokens) {
        let mut current = self.0.theme.lock().unwrap_or_else(PoisonError::into_inner);
        if current.as_ref() == Some(&theme) {
            return;
        }
        *current = Some(theme);
        drop(current);
        self.wake();
    }
}

/// A Blitz event as the view's input, in its logical pixels.
fn view_input(event: &UiEvent) -> Option<ViewInput> {
    Some(match event {
        UiEvent::PointerDown(p) => pointer(p, "start", |x, y| ViewInput::PointerDown {
            x,
            y,
            button: button(p.button),
        }),
        UiEvent::PointerMove(p) => pointer(p, "move", |x, y| ViewInput::PointerMove { x, y }),
        UiEvent::PointerUp(p) => pointer(p, "end", |x, y| ViewInput::PointerUp {
            x,
            y,
            button: button(p.button),
        }),
        // Gone: where it went, outside the view, so nothing is under it.
        UiEvent::PointerLeave(p) => pointer(p, "cancel", |x, y| ViewInput::PointerMove { x, y }),
        UiEvent::PointerCancel(p) => match p.id {
            BlitzPointerId::Finger(_) => pointer(p, "cancel", |_, _| ViewInput::Focus(false)),
            BlitzPointerId::Mouse | BlitzPointerId::Pen => return None,
        },
        UiEvent::Wheel(w) => {
            let (dx, dy) = match w.delta {
                BlitzWheelDelta::Lines(x, y) => (x * LINE, y * LINE),
                BlitzWheelDelta::Pixels(x, y) => (x, y),
            };
            ViewInput::Wheel {
                dx: narrow(dx),
                dy: narrow(dy),
            }
        }
        UiEvent::KeyDown(k) | UiEvent::KeyUp(k) => ViewInput::Key {
            key: k.key.to_string(),
            down: k.state == KeyState::Pressed,
            modifiers: modifiers(k.modifiers),
        },
        UiEvent::Ime(BlitzImeEvent::Preedit(text, _)) => ViewInput::Ime {
            preedit: text.clone(),
            commit: None,
        },
        UiEvent::Ime(BlitzImeEvent::Commit(text)) => ViewInput::Ime {
            preedit: String::new(),
            commit: Some(text.clone()),
        },
        UiEvent::Ime(BlitzImeEvent::Disabled) => ViewInput::Ime {
            preedit: String::new(),
            commit: None,
        },
        // Pasted text is committed text.
        UiEvent::Paste(text) => ViewInput::Ime {
            preedit: String::new(),
            commit: Some(text.clone()),
        },
        UiEvent::Ime(BlitzImeEvent::Enabled | BlitzImeEvent::DeleteSurrounding { .. })
        | UiEvent::AppleStandardKeybinding(_) => return None,
    })
}

/// A pointer event at its element-local point: a finger's as a touch in
/// `phase`, a mouse's or a pen's as `mouse` makes it.
fn pointer(
    p: &BlitzPointerEvent,
    phase: &'static str,
    mouse: impl FnOnce(f32, f32) -> ViewInput,
) -> ViewInput {
    let (x, y) = (p.coords.client_x, p.coords.client_y);
    match p.id {
        BlitzPointerId::Finger(id) => ViewInput::Touch { id, x, y, phase },
        BlitzPointerId::Mouse | BlitzPointerId::Pen => mouse(x, y),
    }
}

const fn button(b: MouseEventButton) -> Button {
    match b {
        MouseEventButton::Secondary => Button::Secondary,
        MouseEventButton::Auxiliary => Button::Middle,
        MouseEventButton::Main | MouseEventButton::Fourth | MouseEventButton::Fifth => {
            Button::Primary
        }
    }
}

/// The held modifiers by [`ViewInput::Key`]'s names: `"Mod"` is ⌘ on a
/// Mac and Ctrl elsewhere.
fn modifiers(m: Modifiers) -> Vec<&'static str> {
    let command = if cfg!(target_os = "macos") {
        Modifiers::META
    } else {
        Modifiers::CONTROL
    };
    [
        (command, "Mod"),
        (Modifiers::SHIFT, "Shift"),
        (Modifiers::ALT, "Alt"),
        (Modifiers::CONTROL, "Ctrl"),
    ]
    .into_iter()
    .filter(|(held, _)| m.contains(*held))
    .map(|(_, name)| name)
    .collect()
}

#[allow(clippy::cast_possible_truncation, clippy::as_conversions)]
const fn narrow(v: f64) -> f32 {
    v as f32
}

/// The shell's theme as a view's tokens: its colours by field name,
/// `_` as `-` (`"ink-2"`, `"accent-soft"`).
#[must_use]
pub fn shell_tokens(theme: &ShellTheme) -> ThemeTokens {
    [
        ("desk", theme.desk),
        ("sheet", theme.sheet),
        ("title-bar", theme.title_bar),
        ("pill", theme.pill),
        ("rule", theme.rule),
        ("active", theme.active),
        ("ink", theme.ink),
        ("ink-2", theme.ink_2),
        ("ink-3", theme.ink_3),
        ("accent", theme.accent),
        ("accent-ink", theme.accent_ink),
        ("danger", theme.danger),
        ("scrim", theme.scrim),
        ("accent-soft", theme.accent_soft),
        ("font", theme.font),
    ]
    .into_iter()
    .fold(ThemeTokens::default(), |t, (name, value)| {
        t.with(name, value)
    })
}

/// What a [`PaintPane`] mounts: its view, made once from the pane's
/// [`ViewCtx`], with the host's typed data and a record of its frames.
#[derive(Clone)]
pub struct PaintMount {
    make: Rc<dyn Fn(&ViewCtx) -> Box<dyn PaintView>>,
    extras: Extras,
    stats: FrameStats,
    budget: Duration,
    wake: PaintWake,
}

impl PartialEq for PaintMount {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.make, &other.make) && self.stats == other.stats
    }
}

impl std::fmt::Debug for PaintMount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaintMount")
            .field("extras", &self.extras)
            .field("budget", &self.budget)
            .finish_non_exhaustive()
    }
}

impl PaintMount {
    /// The view `make` makes, at 60 frames a second's budget.
    pub fn new<V: PaintView>(make: impl Fn(&ViewCtx) -> V + 'static) -> Self {
        Self {
            make: Rc::new(move |ctx| Box::new(make(ctx))),
            extras: Extras::default(),
            stats: FrameStats::new(),
            budget: Duration::from_micros(16_667),
            wake: PaintWake::new(),
        }
    }

    /// The host's typed data and handles ([`ViewCtx::data`]).
    #[must_use]
    pub fn extras(mut self, extras: Extras) -> Self {
        self.extras = extras;
        self
    }

    /// Its frame times kept in `stats` (the host reads them elsewhere).
    #[must_use]
    pub fn stats(mut self, stats: FrameStats) -> Self {
        self.stats = stats;
        self
    }

    /// How long a frame may take ([`FrameClock::budget`]).
    #[must_use]
    pub const fn budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    /// A handle for requesting frames after external state changes.
    #[must_use]
    pub fn wake_handle(&self) -> PaintWake {
        self.wake.clone()
    }
}

/// A pane of the paint kind: `mount`'s view, filling the pane, given the
/// pane's ctx — the app's Action List (run as the person at the app), the
/// shell's theme as tokens, the host's clock and its input.
///
/// Focus is the widget's once it is pressed (or tabbed to); while it has
/// it, the window's IME is on and keys and composed text go to the view.
#[component]
pub fn PaintPane(mount: PaintMount) -> Element {
    let pane = crate::use_pane();
    let actions = try_use_context::<Arc<ActionList>>();
    let theme = architect_shell::try_use_shell().map(|s| s.theme);
    let window = try_use_context::<Arc<dyn ShellProvider>>();
    mount.wake.attach(window.clone());
    mount
        .wake
        .set_theme(theme.as_ref().map(shell_tokens).unwrap_or_default());
    let (widget, input) = use_hook(|| {
        let began = Instant::now();
        let clock = FrameClock::new(mount.budget, move || began.elapsed());
        let actions = actions.map(|a| (*a).clone()).unwrap_or_default();
        let ctx = ViewCtx::new(
            pane.id.clone(),
            KindId::paint(),
            (ViewHost::Blitz, Placement::Pane),
            (actions, ActionCaller::app()),
            clock,
        )
        .with_theme(theme.as_ref().map(shell_tokens).unwrap_or_default())
        .with_extras(mount.extras.clone());
        let input = ctx.input.clone();
        let view = (mount.make)(&ctx);
        let painter = Painter::boxed(ctx, view).with_stats(mount.stats.clone());
        (
            CustomWidgetAttr::new(PaintWidget::with_wake(painter, mount.wake.clone())),
            input,
        )
    });
    let focus = move |on: bool| {
        input.push(ViewInput::Focus(on));
        if let Some(window) = &window {
            window.set_ime_enabled(on);
            window.request_redraw();
        }
    };
    let blur = focus.clone();
    rsx! {
        object {
            style: "display: block; width: 100%; height: 100%; outline: none;",
            tabindex: "0",
            data: widget,
            onfocus: move |_| focus(true),
            onblur: move |_| blur(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PaneId;
    use dioxus::core::NoOpMutations;
    use std::cell::RefCell;
    use std::sync::atomic::AtomicUsize;

    #[derive(Default)]
    struct Window(AtomicUsize);

    impl ShellProvider for Window {
        fn request_redraw(&self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn wake_before_and_after_mount_marks_dirty_and_wakes_the_window() {
        let wake = PaintWake::new();
        wake.wake();
        let window = Arc::new(Window::default());
        wake.attach(Some(window.clone()));
        assert!(wake.0.dirty.swap(false, Ordering::AcqRel));
        assert_eq!(window.0.load(Ordering::Relaxed), 1);
        std::thread::spawn(move || wake.wake()).join().unwrap();
        assert_eq!(window.0.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_reattached_pane_wakes_its_new_window() {
        let wake = PaintWake::new();
        let main = Arc::new(Window::default());
        let popout = Arc::new(Window::default());
        wake.attach(Some(main.clone()));
        wake.wake();
        wake.0.dirty.store(false, Ordering::Release);
        wake.attach(Some(popout.clone()));
        wake.wake();
        assert_eq!(main.0.load(Ordering::Relaxed), 1);
        assert_eq!(popout.0.load(Ordering::Relaxed), 1);
    }

    struct Probe {
        seen: Arc<Mutex<Vec<String>>>,
        wake_on_paint: Option<PaintWake>,
    }

    impl PaintView for Probe {
        fn input(&mut self, ctx: &ViewCtx, _: &ViewInput) -> bool {
            self.seen.lock().unwrap().push(format!(
                "input:{}",
                ctx.theme.get("accent").unwrap_or("none")
            ));
            false
        }

        fn paint(&mut self, ctx: &ViewCtx, _: &mut Scene) {
            self.seen.lock().unwrap().push(format!(
                "paint:{}",
                ctx.theme.get("accent").unwrap_or("none")
            ));
            if let Some(wake) = self.wake_on_paint.take() {
                wake.wake();
            }
        }
    }

    #[test]
    fn theme_and_wake_reach_an_existing_widget_without_losing_a_concurrent_wake() {
        let wake = PaintWake::new();
        let window = Arc::new(Window::default());
        wake.attach(Some(window.clone()));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let ctx = ViewCtx::new(
            PaneId::new("probe"),
            KindId::paint(),
            (ViewHost::Blitz, Placement::Pane),
            (ActionList::new(), ActionCaller::app()),
            FrameClock::new(Duration::from_millis(16), || Duration::ZERO),
        );
        let probe = Probe {
            seen: seen.clone(),
            wake_on_paint: None,
        };
        let mut widget = PaintWidget::with_wake(Painter::new(ctx, probe), wake.clone());
        let geometry = Geometry {
            width: 20.0,
            height: 10.0,
            scale: 2.0,
        };
        let _ = widget.frame(geometry);
        assert!(!widget.needs_redraw());

        wake.set_theme(ThemeTokens::default().with("accent", "red"));
        assert!(widget.needs_redraw());
        widget.handle_event(&UiEvent::Paste("x".into()));
        let _ = widget.frame(geometry);
        assert_eq!(
            &*seen.lock().unwrap(),
            &["paint:none", "input:red", "paint:red"]
        );
        assert!(!widget.needs_redraw());
        let requests = window.0.load(Ordering::Relaxed);
        wake.set_theme(ThemeTokens::default().with("accent", "red"));
        assert_eq!(window.0.load(Ordering::Relaxed), requests);
        assert!(!widget.needs_redraw());

        let ctx = widget.painter.ctx().clone();
        widget.painter = Painter::new(
            ctx,
            Probe {
                seen,
                wake_on_paint: Some(wake.clone()),
            },
        );
        wake.wake();
        let _ = widget.frame(geometry);
        assert!(
            widget.needs_redraw(),
            "a wake during paint must survive that frame"
        );
        let _ = widget.frame(geometry);
        assert!(!widget.needs_redraw());
    }

    thread_local! {
        static THEME: RefCell<Option<Signal<ShellTheme>>> = const { RefCell::new(None) };
        static MOUNT_WAKE: RefCell<Option<PaintWake>> = const { RefCell::new(None) };
    }

    #[component]
    fn TestPaintPane() -> Element {
        let mount = use_hook(|| {
            let mount = PaintMount::new(|_| Probe {
                seen: Arc::new(Mutex::new(Vec::new())),
                wake_on_paint: None,
            });
            MOUNT_WAKE.with(|slot| *slot.borrow_mut() = Some(mount.wake_handle()));
            mount
        });
        rsx! { PaintPane { mount } }
    }

    fn theme_app() -> Element {
        let theme = use_signal(ShellTheme::default);
        THEME.with(|slot| *slot.borrow_mut() = Some(theme));
        let workspace = use_signal(|| {
            let mut workspace = crate::Workspace::new();
            workspace
                .open(
                    crate::Pane::new("paint", "paint-test", "Paint"),
                    crate::Target::Tab,
                )
                .unwrap();
            workspace
        });
        let views = use_hook(|| {
            crate::Views::new().view(crate::View::new(
                "paint-test",
                "Paint",
                |_| rsx! { TestPaintPane {} },
            ))
        });
        let screen = architect_shell::Screen {
            width: 1000.0,
            height: 800.0,
            touch: false,
            insets: Default::default(),
        };
        rsx! {
            architect_shell::AppShell { screen, theme: theme(), tabs: vec![], current: None,
                on_tab: |_| (),
                crate::WorkspaceView { workspace, views }
            }
        }
    }

    #[test]
    fn shell_theme_change_reaches_mounted_pane_without_remounting() {
        let mut dom = VirtualDom::new(theme_app);
        dom.rebuild_in_place();
        let wake = MOUNT_WAKE.with(|slot| slot.borrow().clone().expect("mounted"));
        let original = Arc::clone(&wake.0);
        wake.0.dirty.store(false, Ordering::Release);
        dom.in_runtime(|| {
            THEME.with(|slot| {
                let mut theme = slot.borrow().unwrap();
                theme.write().accent = "#123456";
            })
        });
        dom.render_immediate(&mut NoOpMutations);
        let current = MOUNT_WAKE.with(|slot| slot.borrow().clone().unwrap());
        assert!(Arc::ptr_eq(&original, &current.0));
        assert!(current.0.dirty.load(Ordering::Acquire));
        assert_eq!(
            current
                .0
                .theme
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .get("accent"),
            Some("#123456")
        );
    }
}
