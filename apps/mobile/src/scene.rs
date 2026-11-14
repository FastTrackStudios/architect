//! The UIScene lifecycle, which iOS 27 requires of every app.
//!
//! winit makes its `UIWindow` the pre-scene way (rust-windowing/winit#4224),
//! and iOS 27 never shows a window with no scene. So `Info.plist` names
//! this delegate for the application role, and winit's window is given the
//! app's window scene as soon as both exist: when the scene connects (the
//! delegate), or when the window is made ([`window_created`]), whichever is
//! second.
//!
//! A link the app is opened with (#629) comes here too: a universal link
//! (`https://<node>/s/<code>/…`, an `NSUserActivity` of web browsing) or
//! the app's scheme (`architect://open?url=…`, a URL context), when the
//! scene connects (a cold start) or later. Each is handed to the window
//! as the OS's ([`architect_app::guest::incoming`]), which opens it in the
//! Shared tab with the same checks as any link.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSSet, NSURL, NSUserActivity};
use objc2_ui_kit::{
    UIApplication, UICoordinateSpace, UIOpenURLContext, UIResponder, UIScene,
    UISceneConnectionOptions, UISceneDelegate, UISceneSession, UIView, UIWindow, UIWindowScene,
    UIWindowSceneDelegate,
};

define_class!(
    // SAFETY: a UIResponder subclass with no ivars that overrides nothing
    // of UIResponder's.
    #[unsafe(super(UIResponder))]
    #[thread_kind = MainThreadOnly]
    #[name = "ArchitectSceneDelegate"]
    struct SceneDelegate;

    unsafe impl NSObjectProtocol for SceneDelegate {}

    unsafe impl UISceneDelegate for SceneDelegate {
        #[unsafe(method(scene:willConnectToSession:options:))]
        fn will_connect(
            &self,
            scene: &UIScene,
            _session: &UISceneSession,
            options: &UISceneConnectionOptions,
        ) {
            join(MainThreadMarker::from(scene));
            // A cold start by a link: the scheme's, or a universal link.
            // Both sets are nil on a launch without one, though the
            // bindings declare them non-null (and would panic): read them
            // as optional.
            // SAFETY: `URLContexts` and `userActivities` take no arguments
            // and return an `NSSet` of these element types, or nil.
            let contexts: Option<Retained<NSSet<UIOpenURLContext>>> =
                unsafe { msg_send![options, URLContexts] };
            let activities: Option<Retained<NSSet<NSUserActivity>>> =
                unsafe { msg_send![options, userActivities] };
            if let Some(contexts) = contexts {
                open_contexts(&contexts);
            }
            for activity in activities.iter().flat_map(|a| a.iter()) {
                open_activity(&activity);
            }
        }

        #[unsafe(method(scene:openURLContexts:))]
        fn open_url_contexts(&self, _scene: &UIScene, contexts: &NSSet<UIOpenURLContext>) {
            open_contexts(contexts);
        }

        #[unsafe(method(scene:continueUserActivity:))]
        fn continue_activity(&self, _scene: &UIScene, activity: &NSUserActivity) {
            open_activity(activity);
        }

        #[unsafe(method(sceneDidBecomeActive:))]
        fn did_become_active(&self, scene: &UIScene) {
            join(MainThreadMarker::from(scene));
        }
    }

    unsafe impl UIWindowSceneDelegate for SceneDelegate {}
);

/// Hand `url` to the window, as a link the OS gave the app: a share link
/// or `architect://open?url=<one>` to the Shared tab; a deep link into an
/// action (`architect://<route>`, `https://<domain>/a/<route>`, #186) to
/// its declared route (`architect_app::links`). Anything else is left
/// alone: it never reaches the Shared tab or runs anything.
fn open_url(url: &NSURL) {
    let Some(text) = url.absoluteString() else {
        return;
    };
    let text = text.to_string();
    let shares = architect_app::guest::links_in([text.clone()]);
    if shares.is_empty() {
        // A deep link into an action (#186): `architect://capture?text=…`,
        // or a universal link under `/a/`.
        let _ = architect_app::links::incoming(text);
        return;
    }
    for link in shares {
        architect_app::guest::incoming(link, architect_app::guest::Source::Os);
    }
}

/// The app's scheme: each URL context's URL.
fn open_contexts(contexts: &NSSet<UIOpenURLContext>) {
    for context in contexts.iter() {
        open_url(&context.URL());
    }
}

/// A universal link: a web-browsing activity's URL.
fn open_activity(activity: &NSUserActivity) {
    if activity.activityType().to_string() == "NSUserActivityTypeBrowsingWeb" {
        if let Some(url) = activity.webpageURL() {
            open_url(&url);
        }
    }
}

thread_local! {
    static WINDOW: RefCell<Option<Retained<UIWindow>>> = const { RefCell::new(None) };
}

/// Make the delegate class known to the runtime before UIKit looks it up
/// by the name `Info.plist` gives.
pub fn register() {
    let _ = SceneDelegate::class();
}

/// winit's window has been made; `ui_view` is its view.
pub fn window_created(ui_view: NonNull<c_void>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: a UIKit raw window handle's `ui_view` is a live UIView, read
    // on the main thread.
    let view: &UIView = unsafe { ui_view.cast::<UIView>().as_ref() };
    if let Some(window) = view.window() {
        WINDOW.with(|w| *w.borrow_mut() = Some(window));
        join(mtm);
    }
}

/// Put winit's window in the app's window scene, sized to it, once.
fn join(mtm: MainThreadMarker) {
    let Some(window) = WINDOW.with(|w| w.borrow().clone()) else {
        return;
    };
    if window.windowScene().is_some() {
        return;
    }
    let scenes = UIApplication::sharedApplication(mtm).connectedScenes();
    let Some(scene) = scenes
        .iter()
        .find_map(|s| s.downcast::<UIWindowScene>().ok())
    else {
        return;
    };
    window.setWindowScene(Some(&scene));
    // The scene's own bounds (on an iPad a scene need not fill the screen).
    // Deprecated for `effectiveGeometry`, which the bindings only reach
    // with a main-thread marker per call; the old one still answers.
    #[allow(deprecated)]
    let space = scene.coordinateSpace();
    window.setFrame(UICoordinateSpace::bounds(&*space));
    window.makeKeyAndVisible();
    tracing::info!(ios.scene = "joined", "ios: the window joined its scene");
}
