//! The cross-target WebSocket dial.
//!
//! Same `vox_core::initiator_on(link).establish()` on both targets — what
//! differs is how the *credential* reaches the server, because a browser
//! and a native client have different levers on a WebSocket handshake.
//!
//! ## Where the token goes
//!
//! Not the URL query string, on either target: those land in every proxy
//! and access log on the path.
//!
//! - **Native** controls its handshake request, so the token is an
//!   ordinary `Authorization: Bearer` header — the same channel an HTTP
//!   surface already accepts, and no negotiation to get wrong.
//! - **Browsers** cannot set request headers. The one lever they have is
//!   the subprotocol list, so the token rides as
//!   `vox.bearer.<token>` alongside the `vox.v1` selector.
//!
//! ### Why native offers no subprotocol
//!
//! Symmetry would be nice and costs an outage. tungstenite is stricter
//! than RFC 6455: the spec lets a server that selects no subprotocol omit
//! the response header (§4.2.2 — browsers accept that), but tungstenite
//! treats "I offered, you didn't echo" as a handshake *failure*. A native
//! client offering `vox.bearer.…` therefore cannot talk to any peer that
//! doesn't echo it — an older server, or an ingress that drops the
//! header. `Authorization` has none of that coupling.
//!
//! ### Why the token's charset is checked
//!
//! A subprotocol value must use the RFC 7230 token charset. Session
//! tokens are base64url-no-pad and issuer JWTs are base64url segments
//! joined by `.` — both fit. A token containing anything else is dropped
//! rather than sent as a malformed header that would fail the whole
//! handshake. The dot matters: dropping it for containing one is how an
//! OAuth-redirect sign-in ended up dialling every socket anonymously and
//! seeing "not a member" on every screen.

use super::{ConnectError, Endpoint, RootLane};

/// The subprotocol every dial offers and the server selects.
///
/// Offering *any* subprotocol makes the server's echo mandatory, which is
/// what lets the bearer subprotocol below be added without breaking the
/// handshake.
pub const SUBPROTOCOL: &str = "vox.v1";

/// Prefix of the subprotocol carrying the session token.
pub const BEARER_SUBPROTOCOL_PREFIX: &str = "vox.bearer.";

/// Dial `endpoint` over WebSocket and complete the vox handshake.
///
/// # Errors
///
/// [`ConnectError::NoEndpoint`] for an empty target,
/// [`ConnectError::Dial`] if the socket won't open, and
/// [`ConnectError::Handshake`] if vox won't establish on it.
pub async fn dial(endpoint: Endpoint) -> Result<RootLane, ConnectError> {
    if endpoint.target.is_empty() {
        return Err(ConnectError::NoEndpoint);
    }
    #[cfg(target_arch = "wasm32")]
    let link = browser::dial(&endpoint).await?;
    #[cfg(not(target_arch = "wasm32"))]
    let link = native::dial(&endpoint).await?;

    vox_core::initiator_on(link)
        .establish::<RootLane>()
        .await
        .map_err(|e| ConnectError::Handshake {
            endpoint: endpoint.to_string(),
            reason: format!("{e:?}"),
        })
}

/// The subprotocol list a browser dial offers.
///
/// Always [`SUBPROTOCOL`], plus `vox.bearer.<token>` when a usable token
/// is present. See the module docs for what "usable" means.
#[must_use]
pub fn subprotocols(bearer: Option<&str>) -> Vec<String> {
    let mut protocols = vec![SUBPROTOCOL.to_owned()];
    if let Some(token) = bearer.filter(|t| !t.is_empty() && is_rfc7230_token(t)) {
        protocols.push(format!("{BEARER_SUBPROTOCOL_PREFIX}{token}"));
    }
    protocols
}

/// Does `s` fit the charset a subprotocol value must use?
fn is_rfc7230_token(s: &str) -> bool {
    s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~'))
}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{ConnectError, Endpoint};

    pub(super) async fn dial(
        endpoint: &Endpoint,
    ) -> Result<
        vox_websocket::WsLink<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
        ConnectError,
    > {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest as _;

        let mut request = endpoint
            .target
            .as_str()
            .into_client_request()
            .map_err(|e| ConnectError::Dial {
                endpoint: endpoint.to_string(),
                reason: format!("building request: {e:?}"),
            })?;
        if let Some(token) = endpoint.bearer.as_deref().filter(|t| !t.is_empty()) {
            if let Ok(value) = format!("Bearer {token}").parse() {
                request.headers_mut().insert("authorization", value);
            }
        }
        let (stream, _response) = tokio_tungstenite::connect_async(request)
            .await
            .map_err(|e| ConnectError::Dial {
                endpoint: endpoint.to_string(),
                reason: format!("{e:?}"),
            })?;
        Ok(vox_websocket::WsLink::new(stream))
    }
}

#[cfg(target_arch = "wasm32")]
mod browser {
    use super::{ConnectError, Endpoint, subprotocols};

    /// Cancel-safe browser dial.
    ///
    /// `WsLink::connect`'s own dial phase is not cancel-safe: it attaches
    /// `onopen`/`onerror` closures to the connecting socket and detaches
    /// them only on the success path. On the error path — and, worse,
    /// when the connect future is **dropped mid-dial**, which a reactive
    /// app root does routinely — the closures drop while still attached
    /// to a socket that hasn't finished failing. The browser then
    /// delivers that socket's `error`/`close` into freed memory, which
    /// surfaces as `closure invoked recursively or after being dropped`
    /// and a dead page.
    ///
    /// So the connect-phase closures live in a guard whose `Drop`
    /// detaches them from the socket *first*. Drop order inside one
    /// synchronous Rust drop can't be interleaved with browser event
    /// dispatch, so no event can reach a dropped closure.
    pub(super) async fn dial(endpoint: &Endpoint) -> Result<vox_websocket::WsLink, ConnectError> {
        use std::cell::RefCell;
        use std::rc::Rc;

        use wasm_bindgen::JsCast as _;
        use wasm_bindgen::closure::Closure;

        struct Dial {
            ws: web_sys::WebSocket,
            _onopen: Closure<dyn FnMut()>,
            _onerror: Closure<dyn FnMut(web_sys::Event)>,
            _onclose: Closure<dyn FnMut(web_sys::CloseEvent)>,
            keep_open: bool,
        }
        impl Drop for Dial {
            fn drop(&mut self) {
                // Detach FIRST — after these lines the browser holds no
                // reference into the closures, so dropping them (field
                // drop, immediately after this body) is always safe.
                self.ws.set_onopen(None);
                self.ws.set_onerror(None);
                self.ws.set_onclose(None);
                if !self.keep_open {
                    // Abandoned dial: tear the socket down rather than
                    // leave it connecting into the void.
                    let _ = self.ws.close();
                }
            }
        }

        let failed = |reason: String| ConnectError::Dial {
            endpoint: endpoint.to_string(),
            reason,
        };

        let protocols = js_sys::Array::new();
        for proto in subprotocols(endpoint.bearer.as_deref()) {
            protocols.push(&wasm_bindgen::JsValue::from_str(&proto));
        }
        let ws = web_sys::WebSocket::new_with_str_sequence(&endpoint.target, &protocols)
            .map_err(|e| failed(format!("WebSocket::new: {e:?}")))?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let (tx, rx) = futures::channel::oneshot::channel::<Result<(), String>>();
        let tx = Rc::new(RefCell::new(Some(tx)));

        let settle = {
            let tx = Rc::clone(&tx);
            move |outcome: Result<(), String>| {
                if let Some(tx) = tx.borrow_mut().take() {
                    let _ = tx.send(outcome);
                }
            }
        };

        let onopen = {
            let settle = settle.clone();
            Closure::<dyn FnMut()>::new(move || settle(Ok(())))
        };
        let onerror = {
            let settle = settle.clone();
            Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
                settle(Err("socket error during handshake".to_owned()));
            })
        };
        let onclose =
            Closure::<dyn FnMut(web_sys::CloseEvent)>::new(move |e: web_sys::CloseEvent| {
                settle(Err(format!("closed during handshake: {}", e.code())));
            });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));

        let mut guard = Dial {
            ws,
            _onopen: onopen,
            _onerror: onerror,
            _onclose: onclose,
            keep_open: false,
        };

        match rx.await {
            Ok(Ok(())) => {}
            Ok(Err(reason)) => return Err(failed(reason)),
            Err(_) => return Err(failed("dial cancelled".to_owned())),
        }

        // Success: hand the open socket to `WsLink`, which installs the
        // steady-state handlers it owns.
        guard.keep_open = true;
        Ok(vox_websocket::WsLink::new(guard.ws.clone()))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{BEARER_SUBPROTOCOL_PREFIX, SUBPROTOCOL, subprotocols};

    #[test]
    fn anonymous_dial_offers_only_the_selector() {
        assert_eq!(subprotocols(None), vec![SUBPROTOCOL.to_owned()]);
        assert_eq!(subprotocols(Some("")), vec![SUBPROTOCOL.to_owned()]);
    }

    #[test]
    fn a_jwt_shaped_token_survives_its_dots() {
        // The regression: dropping a token for containing `.` signed
        // every redirect-flow user out at the socket layer.
        let jwt = "eyJhbGci.eyJzdWIi.SflKxwRJ";
        let offered = subprotocols(Some(jwt));
        assert_eq!(offered.len(), 2);
        assert_eq!(
            offered.get(1).unwrap(),
            &format!("{BEARER_SUBPROTOCOL_PREFIX}{jwt}")
        );
    }

    #[test]
    fn a_token_outside_the_header_charset_is_dropped_not_mangled() {
        // Sending it would fail the whole handshake; dialling anonymously
        // at least surfaces as "not a member" rather than a dead socket.
        for bad in ["has space", "has\"quote", "has\nnewline", "has,comma"] {
            assert_eq!(
                subprotocols(Some(bad)),
                vec![SUBPROTOCOL.to_owned()],
                "{bad:?} must not be offered"
            );
        }
    }
}
