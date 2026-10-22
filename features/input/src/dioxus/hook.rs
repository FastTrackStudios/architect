//! A Dioxus hook driving an [`InputProcessor`].

use std::cell::{Ref, RefCell};
use std::rc::Rc;
use std::time::Duration;

use dioxus::prelude::{
    KeyboardEvent, MouseEvent, ReadableExt, Signal, WheelEvent, WritableExt, spawn, use_hook,
    use_signal,
};

use crate::dioxus::convert::{convert_keyboard_event, convert_mouse_event, convert_wheel_event};
use crate::{
    ActionKey, ConfigError, InputCommand, InputContext, InputEvent, InputProcessor, KeymapConfig,
    ModeId, MouseAction,
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

/// A reactive handle on one processor, from [`use_input_processor`].
///
/// Each `handle_*` takes the [`InputContext`] the event happens in: the
/// app owns where that comes from (a signal, a context provider).
pub struct InputHandle<A: 'static = String> {
    processor: Rc<RefCell<InputProcessor<A>>>,
    mirror: Mirror,
    timeout_epoch: Signal<u64>,
}

impl<A: 'static> Clone for InputHandle<A> {
    fn clone(&self) -> Self {
        Self {
            processor: Rc::clone(&self.processor),
            mirror: self.mirror,
            timeout_epoch: self.timeout_epoch,
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
        self.mirror.sync(&self.processor.borrow());
        self.schedule_timeout();
    }

    /// Arm the sequence timeout when a sequence is pending. Each call
    /// bumps an epoch, so only the newest timer fires.
    fn schedule_timeout(&self) {
        let mut timeout_epoch = self.timeout_epoch;
        let epoch = timeout_epoch.peek().wrapping_add(1);
        timeout_epoch.set(epoch);
        let (needs_timeout, timeout) = {
            let processor = self.processor.borrow();
            (processor.needs_timeout(), processor.timeout_duration())
        };
        if !needs_timeout {
            return;
        }
        let processor = Rc::clone(&self.processor);
        let mirror = self.mirror;
        spawn(async move {
            sleep(timeout).await;
            if *timeout_epoch.peek() != epoch {
                return;
            }
            let _unhandled = processor.borrow_mut().timeout_expired();
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
    let timeout_epoch = use_signal(|| 0_u64);
    InputHandle {
        processor,
        mirror: Mirror {
            mode,
            pending,
            recording,
        },
        timeout_epoch,
    }
}
