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
//! Where the keyboard is the screen's own (iOS, [`install_keyboard`]), it
//! comes up only when the person asks for it: a tap that ends on a field
//! (not a scroll that starts on one, [`TapStroke`]), the editing bar's
//! keyboard button, or [`show_keyboard`] from an app that opens a field
//! on purpose (the capture composer). A field focused any other way (an
//! `autofocus`, a press that becomes a scroll) has the focus and no
//! keyboard. [`hide_keyboard`] puts it away whatever has the focus.
//!
//! [`AppShell`]: super::AppShell

#[cfg(target_os = "ios")]
mod ios;

use std::sync::atomic::{AtomicBool, Ordering};
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

    /// What a phone's foot shows. Where the keyboard is the screen's own
    /// (`soft`), the editing bar rides it only while it is up: a field
    /// with the focus and the keyboard down keeps the action bar and the
    /// tabs, with the keyboard button to bring it up. Elsewhere (a desk
    /// window standing in for a phone, a hardware keyboard's host) a typed
    /// field has the editing bar, as [`Self::editing`] says.
    #[must_use]
    pub fn foot(&self, soft: bool) -> Foot {
        if !soft {
            return if self.editing() {
                Foot::Editing
            } else {
                Foot::Tabs
            };
        }
        if self.height > 0.0 {
            Foot::Editing
        } else if self.typing {
            Foot::TabsWithKeyboardButton
        } else {
            Foot::Tabs
        }
    }
}

/// What a phone's foot shows ([`Keyboard::foot`]).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Foot {
    /// The action bar over the tabs.
    Tabs,
    /// The same, with a button that brings the keyboard up for the field
    /// that has the focus.
    TabsWithKeyboardButton,
    /// The editing bar, riding the keyboard.
    Editing,
}

/// How far a finger may move between its press and its release and still
/// be a tap, points (Blitz's own tap slop, about `UIKit`'s).
pub const TAP_SLOP: f64 = 10.0;

/// One finger's stroke as the shell's root hears it: where it went down,
/// and whether it has gone further than a tap does (a scroll, a swipe).
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct TapStroke {
    down: Option<(f64, f64)>,
    moved: bool,
}

impl TapStroke {
    /// A press at `(x, y)`: a new stroke.
    pub const fn press(&mut self, x: f64, y: f64) {
        self.down = Some((x, y));
        self.moved = false;
    }

    /// The finger at `(x, y)`: past [`TAP_SLOP`] from its press, the
    /// stroke is a scroll from then on, even if it comes back.
    pub fn at(&mut self, x: f64, y: f64) {
        if let Some((x0, y0)) = self.down
            && ((x - x0).abs() > TAP_SLOP || (y - y0).abs() > TAP_SLOP)
        {
            self.moved = true;
        }
    }

    /// The release at `(x, y)`: whether the stroke was a tap. One with no
    /// press heard (a child kept it) is taken for one: Blitz sends no
    /// click after a scroll, so a click with no stroke was a tap.
    pub fn release(&mut self, x: f64, y: f64) -> bool {
        self.at(x, y);
        let tap = !self.moved;
        *self = Self::default();
        tap
    }
}

/// A root's ear for the keyboard rule: its pointer events make each
/// finger's [`TapStroke`], and its click brings the keyboard up when the
/// stroke was a tap that left a field typed in ([`tap_raises`]).
///
/// [`AppShell`](super::AppShell)'s root has one. An app whose sheets,
/// palettes and overlays are drawn beside the shell (the Architect app's
/// sign-in sheet, settings, ⌘K) gives its own root one too
/// ([`use_keyboard_taps`]): a tap on their fields never reaches the
/// shell's.
#[derive(Clone)]
pub struct KeyboardTaps {
    stroke: std::rc::Rc<std::cell::Cell<TapStroke>>,
    tap: std::rc::Rc<std::cell::Cell<bool>>,
}

/// A [`KeyboardTaps`] for the calling root: wire its `down`, `moved`,
/// `up` and `click` to the root element's `onpointerdown`,
/// `onpointermove`, `onpointerup` and `onclick`.
#[must_use]
pub fn use_keyboard_taps() -> KeyboardTaps {
    use_hook(|| KeyboardTaps {
        stroke: std::rc::Rc::default(),
        tap: std::rc::Rc::new(std::cell::Cell::new(true)),
    })
}

impl KeyboardTaps {
    pub fn down(&self, e: &PointerEvent) {
        let c = e.client_coordinates();
        let mut s = self.stroke.get();
        s.press(c.x, c.y);
        self.stroke.set(s);
    }

    pub fn moved(&self, e: &PointerEvent) {
        let c = e.client_coordinates();
        let mut s = self.stroke.get();
        s.at(c.x, c.y);
        self.stroke.set(s);
    }

    pub fn up(&self, e: &PointerEvent) {
        let c = e.client_coordinates();
        let mut s = self.stroke.get();
        let was_tap = s.release(c.x, c.y);
        self.stroke.set(s);
        self.tap.set(was_tap);
        if !was_tap {
            super::input_debug::note(format!(
                "scroll ended at {:.0},{:.0}: no keyboard",
                c.x, c.y
            ));
        }
    }

    /// The click that follows a release: the keyboard up if it was a tap
    /// and a field has the focus.
    pub fn click(&self) {
        let was_tap = self.tap.replace(true);
        if tap_raises(soft_keyboard(), was_tap, keyboard()) {
            super::input_debug::note("tap on a field: keyboard up");
            show_keyboard();
        }
    }
}

/// Whether a click the shell's root hears brings the keyboard up: where
/// the keyboard is the screen's own, a tap (not a scroll) that ends with
/// a field typed in and the keyboard down.
#[must_use]
pub const fn tap_raises(soft: bool, tap: bool, now: Keyboard) -> bool {
    soft && tap && now.typing && now.height <= 0.0
}

/// Whether the keyboard here is the screen's own, which comes up only when
/// asked ([`install_keyboard`] says so on iOS).
static SOFT: AtomicBool = AtomicBool::new(false);

/// Whether the keyboard is the screen's own: the editing bar rides it only
/// while it is up, and only a tap or a button brings it up.
#[must_use]
pub fn soft_keyboard() -> bool {
    SOFT.load(Ordering::Relaxed)
}

/// Says the keyboard is the screen's own (iOS does at [`install_keyboard`];
/// a test or another host may).
pub fn set_soft_keyboard(on: bool) {
    SOFT.store(on, Ordering::Relaxed);
}

/// Brings the keyboard up for the field that has the focus.
///
/// The action bar's keyboard button, a tap on a field, or an app opening
/// a field on purpose (a composer). A no-op where the keyboard is not the
/// screen's.
pub fn show_keyboard() {
    super::input_debug::note("keyboard: show asked");
    #[cfg(target_os = "ios")]
    ios::raise_keyboard();
}

/// Whether winit's view has the keyboard (iOS; the debug overlay's).
#[cfg(target_os = "ios")]
pub(super) fn has_keyboard() -> Option<bool> {
    ios::has_keyboard()
}

/// Puts the keyboard away, whatever has the focus, until a tap or
/// [`show_keyboard`] asks for it again; the field keeps the focus.
pub fn hide_keyboard() {
    super::input_debug::note("keyboard: hide asked");
    update(|k| k.typing = false);
    #[cfg(target_os = "ios")]
    ios::lower_keyboard();
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
        super::input_debug::note(format!(
            "keyboard: height {:.0} typing {}",
            now.height, now.typing
        ));
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
///
/// It brings no keyboard up by itself: a field focused by an `autofocus`
/// or by a press that became a scroll has the focus and no keyboard
/// ([`show_keyboard`], [`tap_raises`]).
pub fn set_typing(on: bool) {
    update(|k| k.typing = on);
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
    {
        set_soft_keyboard(true);
        ios::install();
    }
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
        hide_keyboard();
    }
}

/// Hears the feed (and, in a browser or a web view, the page's keyboard)
/// while the calling component is mounted.
pub(super) fn use_keyboard_lift() -> KeyboardLift {
    let lift = use_motion(0.0, Spring::SHEET, 0.5);
    let mut now = use_signal(keyboard);
    use_hook(move || {
        // The window is up: winit's view can be gated now.
        #[cfg(target_os = "ios")]
        ios::gate_when_ready();
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

    const UP: Keyboard = Keyboard {
        height: 336.0,
        typing: true,
    };
    const FOCUSED: Keyboard = Keyboard {
        height: 0.0,
        typing: true,
    };

    #[test]
    fn a_scroll_that_starts_on_a_field_is_no_tap() {
        let mut s = TapStroke::default();
        s.press(100.0, 400.0);
        s.at(101.0, 380.0);
        assert!(!s.release(101.0, 380.0), "20pt up is a scroll");
        // Out and back again is still a scroll.
        s.press(100.0, 400.0);
        s.at(100.0, 300.0);
        assert!(!s.release(100.0, 402.0));
        // A fingertip's roll is a tap.
        s.press(100.0, 400.0);
        s.at(104.0, 397.0);
        assert!(s.release(106.0, 395.0));
        // A click whose press a child kept was a tap (Blitz sends none
        // after a scroll).
        assert!(TapStroke::default().release(5.0, 5.0));
    }

    #[test]
    fn only_a_tap_on_a_field_brings_the_keyboard_up() {
        // A scroll over the focused field: no keyboard.
        assert!(!tap_raises(true, false, FOCUSED));
        // A tap with nothing typed in (a button, a row): none.
        assert!(!tap_raises(true, true, Keyboard::DOWN));
        // A tap on the field: it comes up.
        assert!(tap_raises(true, true, FOCUSED));
        // Already up: nothing to do.
        assert!(!tap_raises(true, true, UP));
        // A desk's keyboard is not the screen's.
        assert!(!tap_raises(false, true, FOCUSED));
    }

    #[test]
    fn the_editing_bar_rides_a_screen_keyboard_only_while_it_is_up() {
        assert_eq!(UP.foot(true), Foot::Editing);
        assert_eq!(FOCUSED.foot(true), Foot::TabsWithKeyboardButton);
        assert_eq!(Keyboard::DOWN.foot(true), Foot::Tabs);
        // On its way down after Hide: the field may still say it types.
        let lowering = Keyboard {
            height: 120.0,
            typing: false,
        };
        assert_eq!(lowering.foot(true), Foot::Editing);
        // A desk standing in for a phone: a typed field has the bar.
        assert_eq!(FOCUSED.foot(false), Foot::Editing);
        assert_eq!(Keyboard::DOWN.foot(false), Foot::Tabs);
    }

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
