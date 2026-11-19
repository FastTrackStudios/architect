//! iOS's side of what is heard: the audio session, the lock screen's
//! card and its buttons.
//!
//! - The session is `playback`: heard with the ringer off, and on with
//!   the screen locked or the app behind another, given `audio` in the
//!   Info.plist's `UIBackgroundModes`. It is made active at the first
//!   play ([`activate_session`]), not at the app's start.
//! - The card (`MPNowPlayingInfoCenter`): the item's title, who it is by,
//!   the source's name, its art, its length and where it is; the system
//!   counts on from there at the rate it is told.
//! - The buttons (`MPRemoteCommandCenter`): play, pause, the toggle a
//!   headset sends, next, previous and the scrubber, each the app's own
//!   press on the deck.
//!
//! Everything is sent by name (`msg_send!`): the frameworks are linked,
//! their bindings crates are not needed for a dozen calls.

use std::path::PathBuf;
use std::sync::OnceLock;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, Sel};
use objc2::{ClassType, class, define_class, msg_send, sel};
use objc2_foundation::{NSNumber, NSString};
use serde_json::json;

use crate::deck::Caller;

#[link(name = "AVFAudio", kind = "framework")]
unsafe extern "C" {
    static AVAudioSessionCategoryPlayback: &'static NSString;
}

#[link(name = "MediaPlayer", kind = "framework")]
unsafe extern "C" {
    static MPMediaItemPropertyTitle: &'static NSString;
    static MPMediaItemPropertyArtist: &'static NSString;
    static MPMediaItemPropertyAlbumTitle: &'static NSString;
    static MPMediaItemPropertyPlaybackDuration: &'static NSString;
    static MPMediaItemPropertyArtwork: &'static NSString;
    static MPNowPlayingInfoPropertyElapsedPlaybackTime: &'static NSString;
    static MPNowPlayingInfoPropertyPlaybackRate: &'static NSString;
}

unsafe extern "C" {
    /// libdispatch's main queue (`dispatch_get_main_queue()` is a macro
    /// over its address).
    static _dispatch_main_q: std::ffi::c_void;
    fn dispatch_async_f(
        queue: *const std::ffi::c_void,
        context: *mut std::ffi::c_void,
        work: extern "C" fn(*mut std::ffi::c_void),
    );
}

/// `work` on the main thread, soon: the lock screen's card and buttons
/// are the app's only when they are set from there.
fn on_main(work: impl FnOnce() + Send + 'static) {
    extern "C" fn run(context: *mut std::ffi::c_void) {
        // SAFETY: `context` is the box `on_main` leaked for this one call.
        let work = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce() + Send>>()) };
        work();
    }
    let boxed: Box<Box<dyn FnOnce() + Send>> = Box::new(Box::new(work));
    // SAFETY: the main queue lives for the process; `run` takes the box
    // back exactly once.
    unsafe {
        dispatch_async_f(
            &raw const _dispatch_main_q,
            Box::into_raw(boxed).cast(),
            run,
        );
    }
}

/// What the lock screen shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Option<f64>,
    pub time: f64,
    pub playing: bool,
    /// The item's art, a file on this device.
    pub art: Option<PathBuf>,
}

/// The session as a player's, active: called before the device is opened.
pub fn activate_session() {
    // SAFETY: `sharedInstance` returns the app's session; `setCategory:
    // error:` and `setActive:error:` take an NSString and a BOOL and an
    // optional error out-pointer, passed null.
    unsafe {
        let session: *mut AnyObject = msg_send![class!(AVAudioSession), sharedInstance];
        if session.is_null() {
            return;
        }
        let none: *mut *mut AnyObject = std::ptr::null_mut();
        let set: bool =
            msg_send![session, setCategory: AVAudioSessionCategoryPlayback, error: none];
        let active: bool = msg_send![session, setActive: true, error: none];
        if !(set && active) {
            tracing::warn!(
                audio.session.category = set,
                audio.session.active = active,
                "playhead.audio.session_refused"
            );
        }
    }
}

/// The card for `card`, or none.
pub fn now_playing(card: Option<&Card>) {
    let card = card.cloned();
    on_main(move || show(card.as_ref()));
}

fn show(card: Option<&Card>) {
    // SAFETY: `defaultCenter` returns the app's centre; `nowPlayingInfo`
    // is a nullable NSDictionary of the keys linked above to NSString,
    // NSNumber and MPMediaItemArtwork values, as built here.
    unsafe {
        let centre: *mut AnyObject = msg_send![class!(MPNowPlayingInfoCenter), defaultCenter];
        if centre.is_null() {
            return;
        }
        let Some(card) = card else {
            let none: *mut AnyObject = std::ptr::null_mut();
            let () = msg_send![centre, setNowPlayingInfo: none];
            return;
        };
        let info: Retained<AnyObject> = msg_send![class!(NSMutableDictionary), dictionary];
        let put = |key: &NSString, value: &AnyObject| {
            let () = msg_send![&*info, setObject: value, forKey: key];
        };
        let text = |key: &NSString, value: &str| {
            if !value.is_empty() {
                put(key, &NSString::from_str(value));
            }
        };
        text(MPMediaItemPropertyTitle, &card.title);
        text(MPMediaItemPropertyArtist, &card.artist);
        text(MPMediaItemPropertyAlbumTitle, &card.album);
        if let Some(d) = card.duration {
            put(MPMediaItemPropertyPlaybackDuration, &NSNumber::new_f64(d));
        }
        put(
            MPNowPlayingInfoPropertyElapsedPlaybackTime,
            &NSNumber::new_f64(card.time),
        );
        put(
            MPNowPlayingInfoPropertyPlaybackRate,
            &NSNumber::new_f64(if card.playing { 1.0 } else { 0.0 }),
        );
        if let Some(art) = card.art.as_deref().and_then(artwork) {
            put(MPMediaItemPropertyArtwork, &art);
        }
        let () = msg_send![centre, setNowPlayingInfo: &*info];
    }
}

/// The image at `file` as the card's artwork.
fn artwork(file: &std::path::Path) -> Option<Retained<AnyObject>> {
    let path = NSString::from_str(file.to_str()?);
    // SAFETY: `imageWithContentsOfFile:` takes a path and returns an
    // image or nil; `initWithImage:` takes that image. The initialiser is
    // an old one, so the class is asked for it first.
    unsafe {
        let image: Option<Retained<AnyObject>> =
            msg_send![class!(UIImage), imageWithContentsOfFile: &*path];
        let image = image?;
        let class = class!(MPMediaItemArtwork);
        let can: bool = msg_send![class, instancesRespondToSelector: sel!(initWithImage:)];
        if !can {
            return None;
        }
        let made: *mut AnyObject = msg_send![class, alloc];
        let made: *mut AnyObject = msg_send![made, initWithImage: &*image];
        Retained::from_raw(made)
    }
}

/// The app's own press on the deck's active source.
fn press(action: &str, args: &serde_json::Value) {
    let _ = crate::ask(&Caller::App, action, args);
}

/// `MPRemoteCommandHandlerStatusSuccess`.
const HANDLED: isize = 0;

define_class!(
    // SAFETY: an NSObject subclass with no ivars that overrides nothing.
    #[unsafe(super(NSObject))]
    #[name = "ArchitectPlayheadRemote"]
    struct Remote;

    impl Remote {
        #[unsafe(method(play:))]
        fn play(&self, _event: &AnyObject) -> isize {
            press("PLAYHEAD_PLAY", &json!({}));
            HANDLED
        }

        #[unsafe(method(pause:))]
        fn pause(&self, _event: &AnyObject) -> isize {
            press("PLAYHEAD_PAUSE", &json!({}));
            HANDLED
        }

        #[unsafe(method(toggle:))]
        fn toggle(&self, _event: &AnyObject) -> isize {
            press("PLAYHEAD_TOGGLE", &json!({}));
            HANDLED
        }

        #[unsafe(method(next:))]
        fn next(&self, _event: &AnyObject) -> isize {
            press("PLAYHEAD_NEXT", &json!({}));
            HANDLED
        }

        #[unsafe(method(previous:))]
        fn previous(&self, _event: &AnyObject) -> isize {
            press("PLAYHEAD_PREVIOUS", &json!({}));
            HANDLED
        }

        #[unsafe(method(seek:))]
        fn seek(&self, event: &AnyObject) -> isize {
            // SAFETY: the scrubber's event is an
            // MPChangePlaybackPositionCommandEvent, whose `positionTime`
            // is seconds as a double.
            let t: f64 = unsafe { msg_send![event, positionTime] };
            press("PLAYHEAD_SEEK", &json!({ "t": t }));
            HANDLED
        }
    }
);

/// The buttons' target, kept for the app's life.
struct Kept(Retained<Remote>);
// SAFETY: it has no state; its methods only call the deck, which locks.
unsafe impl Send for Kept {}
// SAFETY: as above.
unsafe impl Sync for Kept {}

static REMOTE: OnceLock<Kept> = OnceLock::new();

/// The lock screen's buttons, each the app's own press: once.
pub fn commands() {
    on_main(register);
}

fn register() {
    let mut first = false;
    let remote = REMOTE.get_or_init(|| {
        first = true;
        // SAFETY: `new` on an NSObject subclass with no ivars.
        Kept(unsafe { msg_send![Remote::class(), new] })
    });
    if !first {
        return;
    }
    let target: &Remote = &remote.0;
    let commands: [(Sel, Sel); 6] = [
        (sel!(playCommand), sel!(play:)),
        (sel!(pauseCommand), sel!(pause:)),
        (sel!(togglePlayPauseCommand), sel!(toggle:)),
        (sel!(nextTrackCommand), sel!(next:)),
        (sel!(previousTrackCommand), sel!(previous:)),
        (sel!(changePlaybackPositionCommand), sel!(seek:)),
    ];
    // SAFETY: `sharedCommandCenter` returns the app's centre; each
    // selector on the left is a property of it returning an
    // MPRemoteCommand, which takes a target and an action of the shape
    // `Remote`'s methods have and can be enabled.
    unsafe {
        // The app takes the system's media buttons (a headset's, the
        // lock screen's): on the main thread, where UIKit is.
        let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
        if !app.is_null() {
            let () = msg_send![app, beginReceivingRemoteControlEvents];
        }
        let centre: *mut AnyObject = msg_send![class!(MPRemoteCommandCenter), sharedCommandCenter];
        if centre.is_null() {
            return;
        }
        for (command, action) in commands {
            let command: *mut AnyObject = msg_send![centre, performSelector: command];
            if command.is_null() {
                continue;
            }
            let () = msg_send![command, setEnabled: true];
            let () = msg_send![command, addTarget: target, action: action];
        }
    }
}
