//! Protocol mounts on [`EngineHost`]: the endpoints an app serves that
//! are adapters rather than capabilities.
//!
//! `/vox` and `/health` were always the host's. These builder methods
//! extend that to the rest of the protocol surface a server needs, each
//! one a typed hook rather than a hand-written axum route:
//!
//! - [`EngineHost::vox_at`]: a vox WebSocket at a parameterised path
//!   (`/org/{slug}/vox`), with a resolver that picks the [`Lane`] per
//!   connection (a handler, and the org its calls run in), served behind
//!   the host's gate with the lane's scope entered outside it.
//! - [`EngineHost::vox_at_permissioned`]: the same, for a resolver that
//!   gates each connection itself (a gate per org).
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

use super::{EngineHost, LaneGate, VOX_SUBPROTOCOL};
use crate::layer::LayerRouter;
use crate::permissions_gate::{Gated, LaneScope, PermissionsGate};

pub use crate::http::{BEARER_SUBPROTOCOL_PREFIX, RequestHead};

/// The path param [`EngineHost::dav`] binds the in-mount path to.
const DAV_PATH: &str = "dav_path";

/// What an [`EngineHost::vox_at`] resolver serves a connection with: the
/// handler, and the [`LaneScope`] its calls run in (an org).
///
/// Not a handler itself: the host puts its gate between the two, the
/// scope outside, so the gate's engine checks every call in the scope.
/// A lane cannot be served, or wrapped in a gate, any other way.
#[derive(Clone)]
pub struct Lane<H, S = ()> {
    handler: H,
    scope: S,
}

impl<H> Lane<H> {
    /// `handler` in no scope.
    pub const fn new(handler: H) -> Self {
        Self { handler, scope: () }
    }

    /// The lane in `scope`: every call on it, the gate's check included,
    /// runs there.
    #[must_use]
    pub fn scoped<S: LaneScope>(self, scope: S) -> Lane<H, S> {
        Lane {
            handler: self.handler,
            scope,
        }
    }
}

/// What an [`EngineHost::vox_at`] resolver may answer with: a [`Lane`], or
/// a bare [`LayerRouter`] (a lane in no scope).
pub trait IntoLane {
    /// The handler the host gates.
    type Handler;
    /// What the lane's calls run in.
    type Scope: LaneScope;
    /// The lane.
    fn into_lane(self) -> Lane<Self::Handler, Self::Scope>;
}

impl<H, S: LaneScope> IntoLane for Lane<H, S> {
    type Handler = H;
    type Scope = S;
    fn into_lane(self) -> Self {
        self
    }
}

impl IntoLane for LayerRouter {
    type Handler = Self;
    type Scope = ();
    fn into_lane(self) -> Lane<Self> {
        Lane::new(self)
    }
}

/// A lane's handler on an [`ungated`](EngineHost::ungated) host: no check,
/// every call still in the lane's scope.
#[derive(Clone)]
struct Unchecked<H, S> {
    handler: H,
    scope: S,
}

impl<H, S> vox::Handler<vox::DriverReplySink> for Unchecked<H, S>
where
    H: vox::Handler<vox::DriverReplySink> + Send + Sync,
    S: LaneScope,
{
    fn args_have_channels(&self, method_id: vox::MethodId) -> bool {
        self.handler.args_have_channels(method_id)
    }

    fn response_wire_shape(&self, method_id: vox::MethodId) -> Option<&'static facet::Shape> {
        self.handler.response_wire_shape(method_id)
    }

    async fn handle(
        &self,
        call: vox::SelfRef<vox::RequestCall<'static>>,
        reply: vox::DriverReplySink,
        schemas: std::sync::Arc<vox::SchemaRecvTracker>,
    ) {
        self.scope
            .enter(self.handler.handle(call, reply, schemas))
            .await;
    }
}

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
    /// (`/org/{slug}/vox`, `/org/{slug}/share/{token}/vox`), behind the
    /// host's gate.
    ///
    /// Each upgrade calls `resolve` with the request's [`RequestHead`]
    /// first. It answers with the [`Lane`] to serve the connection with
    /// (a bare [`LayerRouter`](crate::LayerRouter), or
    /// `Lane::new(router).scoped(org)` for an org's lane), or with
    /// anything [`IntoResponse`] that refuses it (`404` for an org this
    /// host doesn't serve). The host wraps the lane's handler with its
    /// gate and the upgrade's bearer
    /// ([`PermissionsGate::wrap_shared_with_bearer`]), exactly as on
    /// `/vox`, and enters the lane's scope **outside** that gate, so the
    /// gate's engine checks each call in it (`ConfinedEngine` sees the
    /// lane's org). On an [`ungated`](EngineHost::ungated) host it serves
    /// the handler as is, in the scope. The `vox.v1` subprotocol is echoed
    /// as on `/vox`.
    ///
    /// A lane that needs a gate of its own per connection (each org its
    /// own engine, a share link's fixed guest) is
    /// [`vox_at_permissioned`](Self::vox_at_permissioned), not this.
    ///
    /// `vox_at("/vox", …)` replaces the built-in `/vox`, for a host whose
    /// root lane is picked per connection too (the first hosted org, a
    /// token's org). The router given to [`EngineHost::new`] then serves
    /// only iroh peers.
    ///
    /// ```ignore
    /// EngineHost::new(router, gate, bind).vox_at("/org/{slug}/vox", move |head| {
    ///     let orgs = orgs.clone();
    ///     async move {
    ///         let org = orgs.get(head.param("slug").unwrap_or_default())
    ///             .ok_or(StatusCode::NOT_FOUND)?;
    ///         Ok(Lane::new(org.router()).scoped(org.id()))
    ///     }
    /// })
    /// ```
    #[must_use]
    pub fn vox_at<L, E, F, Fut>(self, path: &str, resolve: F) -> Self
    where
        F: Fn(RequestHead) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<L, E>> + Send + 'static,
        E: IntoResponse,
        L: IntoLane,
        L::Handler: vox::Handler<vox::DriverReplySink> + Clone + Send + Sync + 'static,
    {
        let gate = self.gate.clone();
        let upgrade =
            move |params: RawPathParams, uri: Uri, headers: HeaderMap, ws: WebSocketUpgrade| {
                let head = RequestHead::from_raw(&params, headers, &uri);
                let bearer = head.bearer().map(str::to_owned);
                let resolved = resolve(head);
                let gate = gate.clone();
                async move {
                    match (resolved.await.map(IntoLane::into_lane), gate) {
                        (Ok(Lane { handler, scope }), LaneGate::Gated(gate)) => upgrade_to(
                            ws,
                            PermissionsGate::wrap_shared_with_bearer(gate, handler, bearer)
                                .scoped(scope),
                        ),
                        (Ok(Lane { handler, scope }), LaneGate::Ungated) => {
                            upgrade_to(ws, Unchecked { handler, scope })
                        }
                        (Err(refused), _) => refused.into_response(),
                    }
                }
            };
        self.mount_vox(path, any(upgrade))
    }

    /// [`vox_at`](Self::vox_at) for a resolver that gates each connection
    /// itself: it answers with a
    /// [`PermissionedRouter`](crate::permissions_gate::PermissionedRouter) (a gate per org, a
    /// share link's [`StaticPrincipal`](crate::permissions_gate::StaticPrincipal)
    /// lane), or one [`scoped`](crate::permissions_gate::PermissionedRouter::scoped) to the lane's
    /// org, which is served as is, not wrapped in the host's gate a second
    /// time. The type is the guarantee: an ungated handler, or a scope
    /// inside the gate, does not fit.
    ///
    /// ```ignore
    /// EngineHost::new(router, gate, bind).vox_at_permissioned("/org/{slug}/vox", move |head| {
    ///     let orgs = orgs.clone();
    ///     async move {
    ///         let org = orgs.get(head.param("slug").unwrap_or_default())
    ///             .ok_or(StatusCode::NOT_FOUND)?;
    ///         Ok(PermissionsGate::wrap_shared_with_bearer(
    ///             org.gate(), org.router(), head.bearer().map(str::to_owned),
    ///         ).scoped(org.id()))
    ///     }
    /// })
    /// ```
    #[must_use]
    pub fn vox_at_permissioned<G, E, F, Fut>(self, path: &str, resolve: F) -> Self
    where
        F: Fn(RequestHead) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<G, E>> + Send + 'static,
        E: IntoResponse,
        G: Gated,
    {
        let upgrade =
            move |params: RawPathParams, uri: Uri, headers: HeaderMap, ws: WebSocketUpgrade| {
                let resolved = resolve(RequestHead::from_raw(&params, headers, &uri));
                async move {
                    match resolved.await {
                        Ok(handler) => upgrade_to(ws, handler),
                        Err(refused) => refused.into_response(),
                    }
                }
            };
        self.mount_vox(path, any(upgrade))
    }

    /// Mount a vox upgrade route; at `/vox` it replaces the built-in one.
    fn mount_vox(mut self, path: &str, route: axum::routing::MethodRouter) -> Self {
        self.root_vox_resolved |= path == "/vox";
        self.extend(Router::new().route(path, route))
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

/// Accept a vox WebSocket upgrade (`vox.v1` echoed) and serve `handler`
/// on it.
pub(super) fn upgrade_to<H>(ws: WebSocketUpgrade, handler: H) -> Response
where
    H: vox::Handler<vox::DriverReplySink> + Clone + Send + Sync + 'static,
{
    ws.protocols([VOX_SUBPROTOCOL])
        .on_upgrade(move |socket| crate::axum_ws::serve_router(socket, handler))
        .into_response()
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
