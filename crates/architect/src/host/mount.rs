//! Protocol mounts on [`EngineHost`]: the endpoints an app serves that
//! are adapters rather than capabilities.
//!
//! `/vox` and `/health` were always the host's. These builder methods
//! extend that to the rest of the protocol surface a server needs, each
//! one a typed hook rather than a hand-written axum route:
//!
//! - [`EngineHost::vox_at`]: a vox WebSocket at a parameterised path
//!   (`/org/{slug}/vox`), with a resolver that picks the handler, and so
//!   the identity, per connection.
//! - [`EngineHost::dav`]: a `WebDAV` bridge, every verb, outside the
//!   [`finish`](EngineHost::finish) layers (so outside CORS).
//! - [`EngineHost::document`]: a `GET` document at a path, such as a
//!   `/.well-known/…` descriptor or a per-org `/api` description.
//!
//! Every hook sees the request through a [`RequestHead`]: its path
//! params, headers and bearer token.

use std::future::Future;

use axum::Router;
use axum::extract::{RawPathParams, Request, WebSocketUpgrade};
use axum::http::{HeaderMap, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};

use super::{EngineHost, VOX_SUBPROTOCOL};

pub use crate::http::{BEARER_SUBPROTOCOL_PREFIX, RequestHead};

/// The path param [`EngineHost::dav`] binds the in-mount path to.
const DAV_PATH: &str = "dav_path";

/// One `WebDAV` request, as [`EngineHost::dav`] hands it over.
#[derive(Debug)]
pub struct DavRequest {
    /// The path params of the mount (`slug` in `/org/{slug}/dav`), headers
    /// and bearer.
    pub head: RequestHead,
    /// The mount as this request addressed it, params filled in and no
    /// trailing slash: `/org/acme/dav`. What a `WebDAV` server needs as its
    /// prefix to write `href`s.
    pub mount: String,
    /// The path inside the mount, percent-decoded, without a leading
    /// slash: `Songs/a.wav`, or empty for the mount itself.
    pub rel: String,
    /// The whole request, body and verb (`PROPFIND`, `MKCOL`, …) included.
    pub request: Request,
}

impl EngineHost {
    /// Serve vox over a WebSocket at `path`, which may have params
    /// (`/org/{slug}/vox`, `/org/{slug}/share/{token}/vox`).
    ///
    /// Each upgrade calls `resolve` with the request's [`RequestHead`]
    /// first. It answers with the handler to serve the connection with (a
    /// [`LayerRouter`](crate::LayerRouter), or a gated wrapper of one
    /// built for the caller's identity), or with anything
    /// [`IntoResponse`] that refuses it (`404` for an org this host doesn't serve, `401` for a bad
    /// token). The `vox.v1` subprotocol is echoed as on `/vox`.
    ///
    /// `vox_at("/vox", …)` replaces the built-in `/vox`, for a host whose
    /// root lane is picked per connection too (the first hosted org, a
    /// token's org). The router given to [`EngineHost::new`] then serves
    /// only iroh peers.
    ///
    /// ```ignore
    /// EngineHost::new(router, bind).vox_at("/org/{slug}/vox", move |head| {
    ///     let orgs = orgs.clone();
    ///     async move {
    ///         let org = orgs.get(head.param("slug").unwrap_or_default())
    ///             .ok_or(StatusCode::NOT_FOUND)?;
    ///         Ok(org.router_for(head.bearer()))
    ///     }
    /// })
    /// ```
    #[must_use]
    pub fn vox_at<H, E, F, Fut>(mut self, path: &str, resolve: F) -> Self
    where
        F: Fn(RequestHead) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<H, E>> + Send + 'static,
        E: IntoResponse,
        H: vox::Handler<vox::DriverReplySink> + Clone + Send + Sync + 'static,
    {
        let upgrade =
            move |params: RawPathParams, uri: Uri, headers: HeaderMap, ws: WebSocketUpgrade| {
                let head = RequestHead::from_raw(&params, headers, &uri);
                let resolved = resolve(head);
                async move {
                    match resolved.await {
                        Ok(handler) => ws
                            .protocols([VOX_SUBPROTOCOL])
                            .on_upgrade(move |socket| crate::axum_ws::serve_router(socket, handler))
                            .into_response(),
                        Err(refused) => refused.into_response(),
                    }
                }
            };
        self.root_vox_resolved |= path == "/vox";
        self.extend(Router::new().route(path, any(upgrade)))
    }

    /// Serve a `WebDAV` bridge at `mount` (`/org/{slug}/dav`): every verb,
    /// at the mount itself, with a trailing slash (what a file manager
    /// `PROPFIND`s first) and at every path below it.
    ///
    /// The routes sit **outside** the [`finish`](Self::finish) layers and
    /// the cross-origin isolation headers. A CORS layer answers any
    /// `OPTIONS` request itself, and a `WebDAV` client opens a mount with a
    /// bare `OPTIONS` and reads `DAV:` and `Allow:` off the reply. Behind
    /// CORS it would get neither and refuse to mount. `WebDAV` clients are
    /// not browsers, so CORS has nothing to say about them. The handler
    /// authenticates the request itself ([`RequestHead::bearer`], or Basic
    /// auth from the headers).
    ///
    /// `mount` must not be `/`, and must not end in a wildcard: the
    /// in-mount path is bound for you and handed over as
    /// [`DavRequest::rel`].
    #[must_use]
    pub fn dav<F, Fut>(mut self, mount: &str, handle: F) -> Self
    where
        F: Fn(DavRequest) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Response> + Send + 'static,
    {
        let mount = mount.trim_end_matches('/').to_owned();
        let depth = mount.split('/').filter(|s| !s.is_empty()).count();
        let serve = move |params: RawPathParams, request: Request| {
            let mut head = RequestHead::from_raw(&params, request.headers().clone(), request.uri());
            head.method = request.method().clone();
            let rel = head.params.remove(DAV_PATH).unwrap_or_default();
            let mount = addressed_mount(request.uri().path(), depth);
            handle(DavRequest {
                head,
                mount,
                rel,
                request,
            })
        };
        let routes = Router::new()
            .route(&mount, any(serve.clone()))
            .route(&format!("{mount}/"), any(serve.clone()))
            .route(&format!("{mount}/{{*{DAV_PATH}}}"), any(serve));
        self.outside = Some(match self.outside {
            Some(existing) => existing.merge(routes),
            None => routes,
        });
        self
    }

    /// Answer `GET path` with what `describe` returns: a discovery
    /// document (`/.well-known/<app>.json`), a per-org API description
    /// (`/org/{slug}/api`) or a per-org probe (`/org/{slug}/health`).
    ///
    /// `describe` sees the [`RequestHead`], so it can tag the document for
    /// the bearer or refuse an unknown org with an `Err` (any
    /// [`IntoResponse`]). The route sits before the SPA fallback and
    /// inside the [`finish`](Self::finish) layers, like every plugin.
    ///
    /// ```ignore
    /// .document("/.well-known/task-server.json", move |head| {
    ///     let state = state.clone();
    ///     async move { Ok(Json(discovery(&state, head.bearer()).await)) }
    /// })
    /// ```
    #[must_use]
    pub fn document<D, E, F, Fut>(self, path: &str, describe: F) -> Self
    where
        F: Fn(RequestHead) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<D, E>> + Send + 'static,
        D: IntoResponse,
        E: IntoResponse,
    {
        let answer = move |params: RawPathParams, uri: Uri, headers: HeaderMap| {
            let described = describe(RequestHead::from_raw(&params, headers, &uri));
            async move {
                match described.await {
                    Ok(document) => document.into_response(),
                    Err(refused) => refused.into_response(),
                }
            }
        };
        self.extend(Router::new().route(path, get(answer)))
    }
}

/// The first `depth` segments of `path`: the mount as the client
/// addressed it, before the in-mount path.
fn addressed_mount(path: &str, depth: usize) -> String {
    path.split('/').filter(|s| !s.is_empty()).take(depth).fold(
        String::new(),
        |mut mount, segment| {
            mount.push('/');
            mount.push_str(segment);
            mount
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.append(*k, v.parse().unwrap());
        }
        h
    }

    fn bearer(pairs: &[(&'static str, &str)]) -> Option<String> {
        RequestHead::new(BTreeMap::new(), headers(pairs), "/")
            .bearer()
            .map(str::to_owned)
    }

    #[test]
    fn the_bearer_comes_from_the_header_or_the_subprotocol() {
        assert_eq!(bearer(&[]), None);
        assert_eq!(bearer(&[("sec-websocket-protocol", "vox.v1")]), None);
        assert_eq!(
            bearer(&[("authorization", "Bearer abc123")]).as_deref(),
            Some("abc123")
        );
        assert_eq!(
            bearer(&[("sec-websocket-protocol", "vox.v1, vox.bearer.tok-en_9")]).as_deref(),
            Some("tok-en_9")
        );
        assert_eq!(
            bearer(&[
                ("sec-websocket-protocol", "vox.v1"),
                ("sec-websocket-protocol", "vox.bearer.second")
            ])
            .as_deref(),
            Some("second")
        );
        assert_eq!(
            bearer(&[
                ("authorization", "Bearer header"),
                ("sec-websocket-protocol", "vox.bearer.proto")
            ])
            .as_deref(),
            Some("header"),
            "the header wins"
        );
        assert_eq!(bearer(&[("authorization", "Basic dXNlcg==")]), None);
    }

    #[test]
    fn the_addressed_mount_is_the_first_segments() {
        assert_eq!(
            addressed_mount("/org/acme/dav/Songs/a.wav", 3),
            "/org/acme/dav"
        );
        assert_eq!(addressed_mount("/org/acme/dav/", 3), "/org/acme/dav");
        assert_eq!(addressed_mount("/org/acme/dav", 3), "/org/acme/dav");
    }
}
