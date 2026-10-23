//! A Dioxus hook driving an [`InputProcessor`].

use std::cell::{Cell, Ref, RefCell};
use std::rc::Rc;
use std::time::Duration;

use dioxus::core::{Runtime, ScopeId, current_scope_id};
use dioxus::prelude::{
    KeyboardEvent, MouseEvent, ReadableExt, Signal, WheelEvent, WritableExt, spawn, use_hook,
    use_signal,
};

use crate::dioxus::convert::{
    convert_keyboard_event, convert_mouse_event, convert_wheel_event, keyboard_chord,
};
use crate::{
    ActionKey, ConfigError, InputCommand, InputContext, InputEvent, InputProcessor, KeyChord,
    KeymapConfig, ModeId, MouseAction, SequenceTimeout,
};

/// Sleep for the sequence timeout: tokio's timer on native, the browser
/// timer on wasm.
async fn sleep(duration: Duration) {
    #[cfg(not(target_arch = "wasm32"))]
    tokio::time::sleep(duration).await;
    #[cfg(target_arch = "wasm32")]
    gloo_timers::future::TimeoutFuture::new(
        u32::try_from(duration.as_millis()).unwrap_or(u32::MAX),
    )
    .await;
}

/// The processor's observable state, mirrored into signals.
#[derive(Clone, Copy)]
struct Mirror {
    mode: Signal<ModeId>,
    pending: Signal<Option<String>>,
    recording: Signal<bool>,
}

impl Mirror {
    fn sync<A: ActionKey>(self, processor: &InputProcessor<A>) {
        let Self {
            mut mode,
            mut pending,
            mut recording,
        } = self;
        mode.set(processor.current_mode().clone());
        pending.set(processor.pending_display());
        recording.set(processor.is_recording_macro());
    }
}

/// The Dioxus runtime and scope the handle was made in, so a call from
/// outside the runtime (a `web_sys` document listener) can re-enter it.
#[derive(Clone)]
struct Home {
    runtime: Rc<Runtime>,
    scope: ScopeId,
}

impl Home {
    fn here() -> Self {
        Self {
            runtime: Runtime::current(),
            scope: current_scope_id(),
        }
    }

    fn enter<O>(&self, f: impl FnOnce() -> O) -> O {
        self.runtime.in_scope(self.scope, f)
    }
}

/// A reactive handle on one processor, from [`use_input_processor`].
///
/// Each `handle_*` takes the [`InputContext`] the event happens in: the
/// app owns where that comes from (a signal, a context provider).
///
/// Every method may be called from outside the Dioxus runtime, e.g. a
/// capture-phase `web_sys` listener that must `preventDefault()` in the
/// same tick: the handle re-enters the runtime it was created in to
/// update its signals and arm the sequence timeout.
pub struct InputHandle<A: 'static = String> {
    processor: Rc<RefCell<InputProcessor<A>>>,
    mirror: Mirror,
    home: Home,
    /// The last sequence timer armed, so an event that leaves the same
    /// ticket (a mouse move mid-sequence) does not arm a second one.
    armed: Rc<Cell<Option<SequenceTimeout>>>,
}

impl<A: 'static> Clone for InputHandle<A> {
    fn clone(&self) -> Self {
        Self {
            processor: Rc::clone(&self.processor),
            mirror: self.mirror,
            home: self.home.clone(),
            armed: Rc::clone(&self.armed),
        }
    }
}

impl<A: 'static> PartialEq for InputHandle<A> {
    fn eq(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.processor, &other.processor)
    }
}

impl<A: ActionKey + 'static> InputHandle<A> {
    /// Rebuild the processor from `config`; on error nothing changes.
    pub fn reload_config(&self, config: KeymapConfig) -> Result<(), ConfigError> {
        self.processor.borrow_mut().reload_config(config)?;
        self.after_event();
        Ok(())
    }

    /// The host saw `chord` released. Hosts that see keyups should call
    /// this: without releases a second press of the same key reads as OS
    /// auto-repeat, so `space space` never fires. Returns `true` when a
    /// lingering which-key overlay should hide (see
    /// [`InputProcessor::notify_key_release`]).
    #[must_use = "`true` means hide the which-key overlay"]
    pub fn notify_key_release(&self, chord: &KeyChord) -> bool {
        let hide = self.processor.borrow_mut().notify_key_release(chord);
        self.after_event();
        hide
    }

    /// Report a DOM `keyup`: [`notify_key_release`](Self::notify_key_release)
    /// for its chord. Keys the processor has no code for return `false`.
    #[must_use = "`true` means hide the which-key overlay"]
    pub fn handle_key_release(&self, e: &KeyboardEvent) -> bool {
        keyboard_chord(e).is_some_and(|chord| self.notify_key_release(&chord))
    }

    /// Process a keyboard event. Keys the processor has no code for give
    /// no commands.
    #[must_use]
    pub fn handle_key(&self, e: &KeyboardEvent, ctx: &InputContext) -> Vec<InputCommand<A>> {
        convert_keyboard_event(e).map_or_else(Vec::new, |event| self.process_event(event, ctx))
    }

    /// Process a mouse event as `action`.
    #[must_use]
    pub fn handle_mouse(
        &self,
        e: &MouseEvent,
        action: MouseAction,
        ctx: &InputContext,
    ) -> Vec<InputCommand<A>> {
        self.process_event(convert_mouse_event(e, action), ctx)
    }

    /// Process a wheel event.
    #[must_use]
    pub fn handle_wheel(&self, e: &WheelEvent, ctx: &InputContext) -> Vec<InputCommand<A>> {
        self.process_event(convert_wheel_event(e), ctx)
    }

    /// Process an already-converted event.
    #[must_use]
    pub fn process_event(&self, event: InputEvent, ctx: &InputContext) -> Vec<InputCommand<A>> {
        let commands = self.processor.borrow_mut().process(event, ctx);
        self.after_event();
        commands
    }

    /// The current mode (reactive).
    #[must_use]
    pub fn current_mode(&self) -> ModeId {
        self.mirror.mode.read().clone()
    }

    /// The pending sequence display (reactive).
    #[must_use]
    pub fn pending_display(&self) -> Option<String> {
        self.mirror.pending.read().clone()
    }

    /// Whether a macro is recording (reactive).
    #[must_use]
    pub fn is_recording(&self) -> bool {
        *self.mirror.recording.read()
    }

    /// Borrow the processor for introspection (keymaps, which-key).
    #[must_use]
    pub fn processor(&self) -> Ref<'_, InputProcessor<A>> {
        self.processor.borrow()
    }

    fn after_event(&self) {
        self.home.enter(|| {
            self.mirror.sync(&self.processor.borrow());
            self.schedule_timeout();
        });
    }

    /// Arm a timer for the pending sequence's ticket, once per ticket. A
    /// stale ticket expires nothing, so earlier timers need no cancelling.
    fn schedule_timeout(&self) {
        let ticket = self.processor.borrow().sequence_timeout();
        if self.armed.replace(ticket) == ticket {
            return;
        }
        let Some(ticket) = ticket else {
            return;
        };
        let processor = Rc::clone(&self.processor);
        let mirror = self.mirror;
        spawn(async move {
            sleep(ticket.after()).await;
            let _unhandled = processor.borrow_mut().expire_sequence(ticket);
            mirror.sync(&processor.borrow());
        });
    }
}

/// Own a processor for the component's lifetime and expose it as a
/// reactive [`InputHandle`]. `init` runs once, on first render.
pub fn use_input_processor<A: ActionKey + 'static>(
    init: impl FnOnce() -> InputProcessor<A>,
) -> InputHandle<A> {
    let processor = use_hook(move || Rc::new(RefCell::new(init())));
    let mode = use_signal(|| processor.borrow().current_mode().clone());
    let pending = use_signal(|| processor.borrow().pending_display());
    let recording = use_signal(|| processor.borrow().is_recording_macro());
    let home = use_hook(Home::here);
    let armed = use_hook(|| Rc::new(Cell::new(None)));
    InputHandle {
        processor,
        mirror: Mirror {
            mode,
            pending,
            recording,
        },
        home,
        armed,
    }
}
