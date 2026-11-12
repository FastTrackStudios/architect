//! iOS: the keyboard's height from UIKit, and a web view's own keyboard
//! bar taken away.
//!
//! - **The height**: `UIKeyboardWillChangeFrameNotification` comes as the
//!   keyboard starts to move (up, down, or a new height with the
//!   predictions row), with the frame it will settle at. The shell eases
//!   after it.
//! - **One bar, not two**: a web view puts its own input accessory bar
//!   (previous field, next field, Done) on every text field it focuses,
//!   under the shell's. WebKit's content view (`WKContentView`, the
//!   responder that owns the keyboard) is made to answer
//!   `inputAccessoryView` with nil, as Obsidian's iOS app does. The method
//!   is replaced on that class alone, so no other responder changes; a
//!   Blitz app has no such class, and nothing changes.
//! - **A field focused from code raises the keyboard**: WebKit focuses a
//!   field from code without a keyboard unless a tap did it, so
//!   `_elementDidFocus:…` is told the person is interacting, as Cordova's
//!   `KeyboardDisplayRequiresUserAction = false`. Blitz focuses an
//!   `autofocus` field before the field is one it types in, so it never
//!   asks for the keyboard: when the text focus arrives with no keyboard
//!   ([`raise_keyboard`]), winit's view (which takes the typing) is made
//!   the first responder, as Blitz's own focus does.
//!
//! - **The keyboard comes up when asked** (`super`'s rule): Blitz asks
//!   winit's view to become the first responder whenever a field takes
//!   the focus, which a press that becomes a scroll and an `autofocus` do.
//!   `-[WinitUIView becomeFirstResponder]` is made to refuse unless the
//!   gate is open: [`raise_keyboard`] opens it (a tap on a field, the
//!   keyboard button, an app opening a composer) for a moment, and
//!   [`lower_keyboard`] shuts it and resigns, so Hide works even where
//!   Blitz still holds the field's focus (winit then thinks its IME is on
//!   and never asks again; the next tap raises it from here).
//!
//! Moved here from Task (`task:apps/mobile/src/keyboard_bar.rs`).

use std::ptr::NonNull;
use std::sync::{Mutex, Once, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::ffi::class_replaceMethod;
use objc2::runtime::{AnyClass, AnyObject, Bool, Imp, Sel};
use objc2::sel;
use objc2_foundation::{NSNotification, NSNotificationCenter, NSOperationQueue, NSValue};
use objc2_ui_kit::{
    NSValueUIGeometryExtensions, UIApplication, UIKeyboardFrameEndUserInfoKey,
    UIKeyboardWillChangeFrameNotification, UIScreen, UIWindowScene,
};

pub(super) fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        hear_keyboard();
        web_view_keyboard();
        gate_first_responder();
    });
}

/// How long the gate stays open after a tap or a button asked: long
/// enough for the field a tap opens (a composer's `autofocus`) to take the
/// keyboard on the next frames.
const GATE_OPEN: Duration = Duration::from_millis(800);

/// Until when winit's view may become the first responder.
static GATE: Mutex<Option<Instant>> = Mutex::new(None);

fn gate_open() -> bool {
    GATE.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some_and(|until| Instant::now() < until)
}

fn set_gate(open: bool) {
    *GATE.lock().unwrap_or_else(PoisonError::into_inner) =
        open.then(|| Instant::now() + GATE_OPEN);
}

/// `UIResponder`'s `becomeFirstResponder`, which winit's view inherits.
static BECOME: OnceLock<usize> = OnceLock::new();

type Become = unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> Bool;

/// `-[WinitUIView becomeFirstResponder]`: the inherited one, while the
/// gate is open.
unsafe extern "C-unwind" fn gated_become(this: *mut AnyObject, cmd: Sel) -> Bool {
    if !gate_open() {
        crate::shell::input_debug::note("keyboard: focus without a tap, no keyboard");
        tracing::info!(
            ios.keyboard = "refused",
            "ios: a field took the focus without a tap; no keyboard"
        );
        return Bool::NO;
    }
    let Some(&imp) = BECOME.get() else {
        return Bool::NO;
    };
    // SAFETY: `imp` is UIResponder's own implementation of the selector,
    // read from the superclass before it was overridden here.
    unsafe {
        let inherited: Become = std::mem::transmute(imp);
        inherited(this, cmd)
    }
}

/// Puts the gate in front of winit's view taking the keyboard, once:
/// winit registers its view's class with its first view, so before the
/// window exists (`install`, before launch) there is nothing to gate yet,
/// and the shell asks again when it mounts ([`gate_when_ready`]).
fn gate_first_responder() {
    static GATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if GATED.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some(cls) = AnyClass::get(c"WinitUIView") else {
        tracing::info!(ios.keyboard = "not-yet", "ios: no WinitUIView yet; gated when the shell mounts");
        return;
    };
    GATED.store(true, std::sync::atomic::Ordering::Relaxed);
    let Some(sup) = cls.superclass() else { return };
    let sel = sel!(becomeFirstResponder);
    // The one a call on the superclass runs (UIResponder's), looked up
    // through its superclasses.
    let Some(inherited) = sup.instance_method(sel).map(objc2::runtime::Method::implementation)
    else {
        return;
    };
    let _ = BECOME.set(inherited as usize);
    // SAFETY: the replacement has the selector's signature (`- (BOOL)
    // becomeFirstResponder`, `B@:`) and forwards to the inherited one.
    unsafe {
        let imp: Imp = std::mem::transmute::<Become, Imp>(gated_become);
        class_replaceMethod(
            std::ptr::from_ref(cls).cast_mut(),
            sel,
            imp,
            c"B@:".as_ptr(),
        );
    }
    tracing::info!(ios.keyboard = "gated", "ios: the keyboard comes up on a tap");
}

/// The gate, now that the window (and winit's view class) exists.
pub(super) fn gate_when_ready() {
    gate_first_responder();
}

/// The view that takes the typing: the key window's root view (winit's).
fn typing_view(mtm: MainThreadMarker) -> Option<objc2::rc::Retained<objc2_ui_kit::UIView>> {
    UIApplication::sharedApplication(mtm)
        .connectedScenes()
        .iter()
        .filter_map(|s| s.downcast::<UIWindowScene>().ok())
        .find_map(|s| s.keyWindow())
        .and_then(|w| w.rootViewController())
        .and_then(|c| c.view())
}

/// Puts the keyboard away and shuts the gate: Hide.
pub(super) fn lower_keyboard() {
    set_gate(false);
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(view) = typing_view(mtm) {
        let was = view.isFirstResponder();
        let resigned = view.resignFirstResponder();
        // Whatever else in the window holds it.
        let _ = view.endEditing(true);
        crate::shell::input_debug::note(format!(
            "keyboard: lowered (was first responder {was}, resigned {resigned})"
        ));
        tracing::info!(ios.keyboard = "lowered", was, resigned, "ios: the keyboard is put away");
    }
}

/// Whether winit's view has the keyboard now (the debug overlay's).
pub(super) fn has_keyboard() -> Option<bool> {
    let mtm = MainThreadMarker::new()?;
    typing_view(mtm).map(|v| v.isFirstResponder())
}

/// Feeds the keyboard's height from UIKit's notifications.
fn hear_keyboard() {
    let block = RcBlock::new(|note: NonNull<NSNotification>| {
        // SAFETY: UIKit hands the block a live notification.
        let note = unsafe { note.as_ref() };
        if let Some(height) = settled_height(note) {
            super::set_keyboard_height(height);
        }
    });
    let center = NSNotificationCenter::defaultCenter();
    // SAFETY: the name is UIKit's constant; the block runs on the main
    // queue (it reads the screen), and only reads the notification.
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(
            Some(UIKeyboardWillChangeFrameNotification),
            None,
            Some(&NSOperationQueue::mainQueue()),
            &block,
        )
    };
    // Heard for the life of the process.
    std::mem::forget(token);
    tracing::info!(ios.keyboard = "heard", "ios: the shell hears the keyboard");
}

/// How far up from the screen's bottom the keyboard will reach, points.
fn settled_height(note: &NSNotification) -> Option<f64> {
    let mtm = MainThreadMarker::new()?;
    let info = note.userInfo()?;
    // SAFETY: UIKit's constant key.
    let key = unsafe { UIKeyboardFrameEndUserInfoKey };
    let end = info.objectForKey(key)?.downcast::<NSValue>().ok()?;
    // SAFETY: the end frame is an NSValue of a CGRect.
    let frame = unsafe { end.CGRectValue() };
    // The notification's object is the keyboard's screen (iOS 16 on);
    // before that, the main one.
    let screen = note
        .object()
        .and_then(|o| o.downcast::<UIScreen>().ok())
        .unwrap_or_else(|| {
            #[allow(deprecated)]
            UIScreen::mainScreen(mtm)
        });
    Some((screen.bounds().size.height - frame.origin.y).max(0.0))
}

/// The person asked for the keyboard (a tap on a field, the keyboard
/// button, a composer opened): the gate opens for a moment, and the
/// window's view takes the keyboard if it can and has not.
pub(super) fn raise_keyboard() {
    set_gate(true);
    // Not by WebKit's class being there: its accessibility bundle loads it
    // into a Blitz app too. A web view's content view has the keyboard
    // already (`allow_focus_from_code`), and the view under it cannot take
    // it.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if let Some(view) =
        typing_view(mtm).filter(|v| !v.isFirstResponder() && v.canBecomeFirstResponder())
    {
        let raised = view.becomeFirstResponder();
        crate::shell::input_debug::note(format!("keyboard: raised ({raised})"));
        tracing::info!(
            ios.keyboard = "raised",
            raised,
            "ios: the keyboard came up, asked for"
        );
    }
}

/// The web view's own bar off, and `focus()` raising the keyboard.
fn web_view_keyboard() {
    let Some(cls) = AnyClass::get(c"WKContentView") else {
        // A Blitz app: no web view, nothing to take away.
        return;
    };
    // SAFETY: the implementation matches the selector's signature
    // (`- (UIView *)inputAccessoryView`, type encoding `@@:`), and nil is
    // what the property's contract allows.
    unsafe {
        let imp: Imp = std::mem::transmute::<
            unsafe extern "C-unwind" fn(*mut AnyObject, Sel) -> *mut AnyObject,
            Imp,
        >(no_accessory);
        class_replaceMethod(
            std::ptr::from_ref(cls).cast_mut(),
            sel!(inputAccessoryView),
            imp,
            c"@@:".as_ptr(),
        );
    }
    tracing::info!(
        ios.keyboard = "web-bar-off",
        "ios: the web view's keyboard bar is off"
    );
    allow_focus_from_code(cls);
}

/// `-[WKContentView inputAccessoryView]`: none.
unsafe extern "C-unwind" fn no_accessory(_this: *mut AnyObject, _cmd: Sel) -> *mut AnyObject {
    std::ptr::null_mut()
}

static ORIGINAL: OnceLock<usize> = OnceLock::new();

type ElementDidFocus = unsafe extern "C-unwind" fn(
    *mut AnyObject,
    Sel,
    *mut std::ffi::c_void,
    Bool,
    Bool,
    u64,
    *mut AnyObject,
);

/// `-[WKContentView _elementDidFocus:userIsInteracting:…]`, told the
/// person is interacting.
unsafe extern "C-unwind" fn element_did_focus(
    this: *mut AnyObject,
    cmd: Sel,
    element: *mut std::ffi::c_void,
    _user_is_interacting: Bool,
    blur_previous: Bool,
    activity_changes: u64,
    user_object: *mut AnyObject,
) {
    let Some(&imp) = ORIGINAL.get() else { return };
    // SAFETY: `imp` is the method's own implementation, saved when it was
    // replaced; the arguments pass through unchanged but one.
    unsafe {
        let original: ElementDidFocus = std::mem::transmute(imp);
        original(
            this,
            cmd,
            element,
            Bool::YES,
            blur_previous,
            activity_changes,
            user_object,
        );
    }
}

fn allow_focus_from_code(cls: &AnyClass) {
    for m in cls.instance_methods().iter() {
        let name = m.name().name().to_bytes();
        // `self`, `_cmd` and the five arguments.
        if name.starts_with(b"_elementDidFocus:userIsInteracting:") && m.arguments_count() == 7 {
            // SAFETY: the replacement has the method's signature and
            // forwards to the original it replaces.
            unsafe {
                let replacement: Imp =
                    std::mem::transmute::<ElementDidFocus, Imp>(element_did_focus);
                let original = m.set_implementation(replacement);
                let _ = ORIGINAL.set(original as usize);
            }
            tracing::info!(
                ios.keyboard = "focus-raises",
                "ios: focus() raises the keyboard"
            );
            return;
        }
    }
    tracing::warn!(
        ios.keyboard = "focus-needs-tap",
        "ios: no _elementDidFocus: on WKContentView; focus needs a tap"
    );
}
