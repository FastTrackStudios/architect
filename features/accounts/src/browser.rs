//! Where a sign-in's page opens: the issuer's own page, in the system's
//! browser, never a page the app draws or a web view it drives.
//!
//! The sign-in is the authorization code with PKCE (`crate::redirect`): the
//! person signs in on the issuer's page (a password, a passkey, a social
//! account; a password manager fills it) and the issuer sends the browser
//! back to the app.
//!
//! - **iOS and iPadOS**: `ASWebAuthenticationSession`, the system's sign-in
//!   sheet over the app's window scene, back on the app's scheme
//!   ([`SCHEME`]), which the sheet hands to the app. It shares Safari's
//!   cookies, so a person already signed in at the issuer only continues.
//!   The sheet's Cancel ends the attempt and the app is as it was.
//! - **Elsewhere** (macOS, Linux, Windows): the default browser
//!   (`webbrowser`), back on a loopback port the app listens on
//!   (`crate::redirect::loopback_redirect`).
//!
//! A page that is not on the issuer's origin is never opened
//! ([`on_issuer`]): the app trusts no other host with a sign-in.

/// The app's own URL scheme (reverse DNS, RFC 8252 §7.1): where iOS's
/// sheet returns (`crate::redirect::SCHEME_REDIRECT`).
pub const SCHEME: &str = "app.fasttrackstudio.architect";

/// Whether `url` is a page of the issuer at `origin` (`https://host[:port]`):
/// the origin itself, or a path or query under it.
#[must_use]
pub fn on_issuer(url: &str, origin: &str) -> bool {
    let origin = origin.trim_end_matches('/');
    if origin.is_empty() || !(origin.starts_with("https://") || origin.starts_with("http://")) {
        return false;
    }
    url == origin
        || url
            .strip_prefix(origin)
            .is_some_and(|rest| rest.starts_with('/') || rest.starts_with('?'))
}

/// Opens `url` for its person to sign in. On iOS `on_done` is given the
/// URL the sheet came back on (the redirect, with its code), or `None`
/// when its person closed it; elsewhere the redirect arrives at the app's
/// loopback port and `on_done` is not called. False when it could not
/// open.
pub(crate) fn open(url: &str, on_done: impl Fn(Option<String>) + Send + 'static) -> bool {
    #[cfg(target_os = "ios")]
    {
        ios::open(url, Box::new(on_done))
    }
    #[cfg(not(target_os = "ios"))]
    {
        let _ = on_done;
        webbrowser::open(url).is_ok()
    }
}

/// Closes what [`open`] opened, where the app can (iOS's sheet): the
/// sign-in is done, failed or cancelled from the app.
#[allow(clippy::missing_const_for_fn, reason = "not const on iOS")]
pub(crate) fn close() {
    #[cfg(target_os = "ios")]
    ios::close();
}

#[cfg(target_os = "ios")]
mod ios {
    use std::cell::RefCell;

    use block2::RcBlock;
    use objc2::rc::{Allocated, Retained};
    use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, NSObject};
    use objc2::{ClassType, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSError, NSOperationQueue, NSString, NSURL};
    use objc2_ui_kit::{UIApplication, UIWindow, UIWindowScene};

    // The system's sign-in sheet.
    #[link(name = "AuthenticationServices", kind = "framework")]
    unsafe extern "C" {}

    define_class!(
        /// Answers the sheet which window it goes over: the app's key
        /// window.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "ArchitectSignInAnchor"]
        struct Anchor;

        impl Anchor {
            #[unsafe(method_id(presentationAnchorForWebAuthenticationSession:))]
            fn anchor(&self, _session: &AnyObject) -> Option<Retained<UIWindow>> {
                key_window(self.mtm())
            }
        }
    );

    impl Anchor {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            // SAFETY: NSObject's plain init.
            unsafe { msg_send![Self::alloc(mtm), init] }
        }
    }

    fn key_window(mtm: MainThreadMarker) -> Option<Retained<UIWindow>> {
        UIApplication::sharedApplication(mtm)
            .connectedScenes()
            .iter()
            .filter_map(|s| s.downcast::<UIWindowScene>().ok())
            .find_map(|s| s.keyWindow())
    }

    /// The sheet on show, and what gives it its window.
    struct Open {
        session: Retained<AnyObject>,
        _anchor: Retained<Anchor>,
    }

    thread_local! {
        static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    }

    /// Runs `f` on the main thread (UIKit's).
    fn on_main(f: impl FnOnce(MainThreadMarker) + 'static) {
        if let Some(mtm) = MainThreadMarker::new() {
            f(mtm);
            return;
        }
        let once = RefCell::new(Some(f));
        let block = RcBlock::new(move || {
            if let (Some(f), Some(mtm)) = (once.borrow_mut().take(), MainThreadMarker::new()) {
                f(mtm);
            }
        });
        // SAFETY: the block runs once on the main queue; what it holds
        // lives in it.
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
    }

    pub(super) fn open(url: &str, on_done: Box<dyn Fn(Option<String>) + Send>) -> bool {
        let url = url.to_owned();
        on_main(move |mtm| {
            close_now();
            let Some(class) = AnyClass::get(c"ASWebAuthenticationSession") else {
                tracing::warn!("accounts: no ASWebAuthenticationSession");
                return;
            };
            let Some(nsurl) = NSURL::URLWithString(&NSString::from_str(&url)) else {
                return;
            };
            // Called when the sheet ends: back on the app's scheme (the
            // redirect), closed by its person (Cancel), or by the app
            // (`close`), which has taken it from `OPEN` first.
            let handler = RcBlock::new(move |url: *mut NSURL, error: *mut NSError| {
                let was_open = OPEN.with(|o| o.borrow_mut().take()).is_some();
                if !was_open {
                    return;
                }
                // SAFETY: UIKit hands a live URL or none.
                let back = unsafe { url.as_ref() }
                    .and_then(NSURL::absoluteString)
                    .map(|s| s.to_string());
                if error.is_null() && back.is_some() {
                    on_done(back);
                } else {
                    tracing::info!("accounts: the sign-in sheet was closed by its person");
                    on_done(None);
                }
            });
            let scheme = NSString::from_str(super::SCHEME);
            // SAFETY: the class's documented initializer; the URL, the
            // scheme and the block outlive the call (the session copies
            // the block).
            let session: Option<Retained<AnyObject>> = unsafe {
                let alloc: Allocated<AnyObject> = msg_send![class, alloc];
                msg_send![alloc, initWithURL: &*nsurl, callbackURLScheme: &*scheme, completionHandler: &*handler]
            };
            let Some(session) = session else { return };
            let anchor = Anchor::new(mtm);
            if let Some(protocol) =
                AnyProtocol::get(c"ASWebAuthenticationPresentationContextProviding")
            {
                // SAFETY: the class implements the protocol's one method.
                unsafe {
                    objc2::ffi::class_addProtocol(
                        std::ptr::from_ref(Anchor::class()).cast_mut(),
                        protocol,
                    );
                }
            }
            // SAFETY: documented setters on a live session; the anchor is
            // kept with it.
            let started: bool = unsafe {
                let _: () = msg_send![&*session, setPresentationContextProvider: &*anchor];
                // Safari's cookies: a person signed in there only approves.
                let _: () = msg_send![&*session, setPrefersEphemeralWebBrowserSession: Bool::NO];
                let ok: Bool = msg_send![&*session, start];
                ok.as_bool()
            };
            tracing::info!(started, "accounts: the sign-in sheet");
            if started {
                OPEN.with(|o| {
                    *o.borrow_mut() = Some(Open {
                        session,
                        _anchor: anchor,
                    });
                });
            }
        });
        true
    }

    fn close_now() {
        if let Some(open) = OPEN.with(|o| o.borrow_mut().take()) {
            // SAFETY: a live session's documented `cancel`.
            let _: () = unsafe { msg_send![&*open.session, cancel] };
        }
    }

    pub(super) fn close() {
        on_main(|_| close_now());
    }
}

#[cfg(test)]
mod tests {
    use super::on_issuer;

    #[test]
    fn only_the_issuers_own_pages_open() {
        let issuer = "https://auth.fasttrackstudio.app";
        assert!(on_issuer(
            "https://auth.fasttrackstudio.app/device?user_code=ABCD-EFGH",
            issuer
        ));
        assert!(on_issuer("https://auth.fasttrackstudio.app", issuer));
        assert!(on_issuer(
            "https://auth.fasttrackstudio.app/",
            "https://auth.fasttrackstudio.app/"
        ));
        // Another host, one that only starts like it, another scheme or port.
        assert!(!on_issuer("https://evil.example/device", issuer));
        assert!(!on_issuer(
            "https://auth.fasttrackstudio.app.evil.example/device",
            issuer
        ));
        assert!(!on_issuer("http://auth.fasttrackstudio.app/device", issuer));
        assert!(!on_issuer(
            "https://auth.fasttrackstudio.app:8443/device",
            issuer
        ));
        assert!(!on_issuer("app.fasttrackstudio.architect:/x", issuer));
        // A local issuer, as the dev stack runs it.
        assert!(on_issuer(
            "http://127.0.0.1:7411/device?user_code=X",
            "http://127.0.0.1:7411"
        ));
        assert!(!on_issuer(
            "http://127.0.0.1:7412/device",
            "http://127.0.0.1:7411"
        ));
        // No issuer: nothing opens.
        assert!(!on_issuer("https://x/", ""));
    }
}
