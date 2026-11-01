//! The on-screen keyboard as the shell hears it: how tall it is, and
//! whether a text field has the focus.
//!
//! Every host reports to one process-wide feed, and every [`AppShell`]
//! hears it:
//!
//! - **iOS** ([`install_keyboard`]): the system's keyboard notifications, the
//!   keyboard's frame as it is about to settle.
//! - **A browser or a web view**: `visualViewport` (the keyboard shrinks
//!   the visual viewport, not the layout), and `focusin` / `focusout` for
//!   the text focus, from a script the shell runs itself.
//! - **Blitz**: the text focus is the document's, which the host forwards:
//!   `dioxus_native::on_text_focus_change(architect_shell::set_typing)`.
//!   A host with its own keyboard source reports it with
//!   [`set_keyboard_height`].
//!
//! [`AppShell`]: super::AppShell

#[cfg(target_os = "ios")]
mod ios;

use std::sync::{Mutex, PoisonError};

use dioxus::prelude::*;
use futures_channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures_util::StreamExt as _;

use crate::motion::{Motion, Spring, use_motion};

/// The keyboard, now.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Keyboard {
    /// How far the on-screen keyboard reaches up from the screen's bottom
    /// edge, points (0 while it is down, or with a hardware keyboard).
    pub height: f64,
    /// Whether a text field has the focus.
    pub typing: bool,
}

impl Keyboard {
    const DOWN: Self = Self {
        height: 0.0,
        typing: false,
    };

    /// Whether the bar is the editing bar: a field is typed in, or the
    /// keyboard is up.
    #[must_use]
    pub fn editing(&self) -> bool {
        self.typing || self.height > 0.0
    }
}

/// The keyboard as last reported, and the shells listening.
struct Feed {
    now: Keyboard,
    listeners: Vec<UnboundedSender<Keyboard>>,
}

static FEED: Mutex<Feed> = Mutex::new(Feed {
    now: Keyboard::DOWN,
    listeners: Vec::new(),
});

fn update(change: impl FnOnce(&mut Keyboard)) {
    let mut feed = FEED.lock().unwrap_or_else(PoisonError::into_inner);
    let before = feed.now;
    change(&mut feed.now);
    let now = feed.now;
    if now != before {
        // A shell that is gone has dropped its end.
        feed.listeners.retain(|tx| tx.unbounded_send(now).is_ok());
    }
}

fn subscribe() -> (Keyboard, UnboundedReceiver<Keyboard>) {
    let (tx, rx) = unbounded();
    let mut feed = FEED.lock().unwrap_or_else(PoisonError::into_inner);
    feed.listeners.push(tx);
    (feed.now, rx)
}

/// Whether a text field has the focus: a host's text focus, reported.
///
/// Its signature is `dioxus_native::on_text_focus_change`'s, so a Blitz
/// host passes it as it is; an app that hears the text focus itself
/// calls it from there.
pub fn set_typing(on: bool) {
    update(|k| k.typing = on);
    #[cfg(target_os = "ios")]
    if on {
        ios::raise_keyboard();
    }
}

/// How far the on-screen keyboard reaches up from the screen's bottom,
/// points; 0 when it is down. iOS and browsers report it themselves.
pub fn set_keyboard_height(height: f64) {
    update(|k| k.height = height.max(0.0));
}

/// The keyboard as last reported.
#[must_use]
pub fn keyboard() -> Keyboard {
    FEED.lock().unwrap_or_else(PoisonError::into_inner).now
}

/// Starts hearing the platform's keyboard, once, before launch.
///
/// iOS: the system's keyboard notifications feed [`set_keyboard_height`];
/// and in a web view (Dioxus mobile, Task's), the web view's own ⌃ ⌄ ✓ bar is taken
/// off every field, since the shell draws the one bar over the keyboard,
/// and a page's own `focus()` raises the keyboard as a tap does. A no-op
/// elsewhere (a browser's keyboard is heard by the shell itself).
// Not `const` on iOS.
#[allow(clippy::missing_const_for_fn)]
pub fn install_keyboard() {
    #[cfg(target_os = "ios")]
    ios::install();
}

/// What the shell's script sends from a browser or a web view: the
/// keyboard's height and whether a text field has the focus.
const WEB_KEYBOARD: &str = r"
const vv = window.visualViewport;
const typing = () => {
  const el = document.activeElement;
  return !!el && (el.isContentEditable || el.tagName === 'TEXTAREA' ||
    (el.tagName === 'INPUT' && !['button','checkbox','radio','range','submit','reset','file','color'].includes(el.type)));
};
const send = () => {
  const kb = vv ? Math.max(0, window.innerHeight - vv.height - vv.offsetTop) : 0;
  dioxus.send([kb, typing()]);
};
if (vv) { vv.addEventListener('resize', send); vv.addEventListener('scroll', send); }
document.addEventListener('focusin', send);
document.addEventListener('focusout', () => setTimeout(send, 0));
send();
await new Promise(() => {});
";

/// The keyboard for the shell: what the feed says, and how far the bar
/// is lifted now, easing after the keyboard.
#[derive(Clone, Copy)]
pub(super) struct KeyboardLift {
    pub(super) now: Signal<Keyboard>,
    // On the motion clock (#145): a film or test drives it like the rest.
    lift: Motion,
}

impl KeyboardLift {
    /// How far the bar is lifted off the screen's bottom, points.
    pub(super) fn lift(self) -> f64 {
        self.lift.value()
    }

    /// The keyboard is put away at once (the bar's own button), ahead of
    /// what the host reports.
    pub(super) fn dismissed(mut self) {
        let mut now = self.now.write();
        now.typing = false;
        drop(now);
        set_typing(false);
    }
}

/// Hears the feed (and, in a browser or a web view, the page's keyboard)
/// while the calling component is mounted.
pub(super) fn use_keyboard_lift() -> KeyboardLift {
    let lift = use_motion(0.0, Spring::SHEET, 0.5);
    let mut now = use_signal(keyboard);
    use_hook(move || {
        let (first, mut heard) = subscribe();
        now.set(first);
        spawn(async move {
            while let Some(k) = heard.next().await {
                now.set(k);
            }
        });
        // Where there is no script to run (Blitz) the first answer is
        // `Unsupported`, and this ends.
        spawn(async move {
            let mut page = document::eval(WEB_KEYBOARD);
            while let Ok((height, typing)) = page.recv::<(f64, bool)>().await {
                update(|k| {
                    k.height = height.max(0.0);
                    k.typing = typing;
                });
            }
        });
    });
    let height = now().height;
    use_effect(use_reactive!(|height| lift.to(height)));
    KeyboardLift { now, lift }
}

#[cfg(test)]
mod tests {
    use super::*;

    // One test: the feed is process-wide.
    #[test]
    fn the_feed_tells_every_listener_each_change_once() {
        let (first, mut a) = subscribe();
        assert_eq!(first, keyboard());
        set_keyboard_height(0.0);
        set_typing(false);
        // Nothing changed: nothing told.
        assert!(a.try_recv().is_err());

        set_typing(true);
        let heard = a.try_recv().ok();
        assert_eq!(
            heard,
            Some(Keyboard {
                height: 0.0,
                typing: true
            })
        );
        assert!(heard.is_some_and(|k| k.editing()), "typing is editing");

        let (_, mut b) = subscribe();
        set_keyboard_height(336.0);
        set_keyboard_height(-4.0);
        assert_eq!(a.try_recv().ok().map(|k| k.height), Some(336.0));
        assert_eq!(b.try_recv().ok().map(|k| k.height), Some(336.0));
        // A height below the screen's edge is no keyboard.
        assert_eq!(a.try_recv().ok().map(|k| k.height), Some(0.0));

        drop(b);
        set_typing(false);
        assert!(!keyboard().editing());
        assert_eq!(
            FEED.lock()
                .unwrap_or_else(PoisonError::into_inner)
                .listeners
                .len(),
            1,
            "a listener that went is let go"
        );
    }
}
