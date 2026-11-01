//! The UIScene lifecycle, which iOS 27 requires of every app.
//!
//! winit makes its `UIWindow` the pre-scene way (rust-windowing/winit#4224),
//! and iOS 27 never shows a window with no scene. So `Info.plist` names
//! this delegate for the application role, and winit's window is given the
//! app's window scene as soon as both exist: when the scene connects (the
//! delegate), or when the window is made ([`window_created`]), whichever is
//! second.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class};
use objc2_ui_kit::{
    UIApplication, UICoordinateSpace, UIResponder, UIScene, UISceneConnectionOptions,
    UISceneDelegate, UISceneSession, UIView, UIWindow, UIWindowScene, UIWindowSceneDelegate,
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
            _options: &UISceneConnectionOptions,
        ) {
            join(MainThreadMarker::from(scene));
        }

        #[unsafe(method(sceneDidBecomeActive:))]
        fn did_become_active(&self, scene: &UIScene) {
            join(MainThreadMarker::from(scene));
        }
    }

    unsafe impl UIWindowSceneDelegate for SceneDelegate {}
);

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
