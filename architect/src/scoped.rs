//! Instance-scoped service mounting — the same `#[architect::rpc]` trait
//! mounted several times on one [`LayerRouter`](crate::LayerRouter), one
//! instance per scope.
//!
//! Vox method ids hash `service.method` names only, so two backends
//! implementing the same trait collide on a merged router (last-merge wins).
//! Scoping disambiguates at dispatch time instead of in the id: the client
//! stamps every call with a `svc-scope` metadata entry
//! ([`ScopeMiddleware`](crate::scoped::ScopeMiddleware)); the router keeps a `(scope, method)` map consulted
//! before the flat one ([`LayerRouter::merge_router_scoped`]). Unscoped
//! services and unscoped calls are untouched — the flat map still serves
//! them — so scoping is purely additive.
//!
//! ```no_run
//! # use architect::{LayerRouter, scoped::ScopeMiddleware};
//! // Server: one RigCore backend per rig, same trait, one router.
//! # let (guitar, keys): (LayerRouter, LayerRouter) = (LayerRouter::new(), LayerRouter::new());
//! let router = LayerRouter::new()
//!     .merge_router_scoped("guitar", guitar)
//!     .merge_router_scoped("keys", keys);
//!
//! // Client: a typed client whose calls carry the scope.
//! // let mut client: RigCoreClient = initiator.establish().await?;
//! // client.caller = client.caller.with_middleware(ScopeMiddleware::new("keys"));
//! ```

/// Metadata key carrying the service-instance scope.
pub const SCOPE_METADATA_KEY: &str = "svc-scope";

/// Client middleware stamping every call with an instance scope. Attach to a
/// generated client's public `caller`:
/// `client.caller = client.caller.with_middleware(ScopeMiddleware::new("keys"))`.
pub struct ScopeMiddleware {
    scope: String,
}

impl ScopeMiddleware {
    pub fn new(scope: impl Into<String>) -> Self {
        Self { scope: scope.into() }
    }
}

impl vox::ClientMiddleware for ScopeMiddleware {
    fn pre<'a, 'call>(
        &'a self,
        _context: &'a vox::ClientContext<'a>,
        request: &'a mut vox::ClientRequest<'call, 'a>,
    ) -> vox::BoxMiddlewareFuture<'a> {
        Box::pin(async move {
            request.push_string_metadata(SCOPE_METADATA_KEY, self.scope.clone());
        })
    }
}

/// Attach a scope to a generated client — the ergonomic wrapper over the
/// generated `with_middleware` (which supplies the service descriptor).
#[macro_export]
macro_rules! scope_client {
    ($client:expr, $scope:expr) => {
        $client.with_middleware($crate::scoped::ScopeMiddleware::new($scope))
    };
}

/// The scope a request carries, if any.
pub fn scope_of(metadata: &vox::Metadata) -> Option<&str> {
    use vox::MetadataExt;
    metadata.meta_str(SCOPE_METADATA_KEY)
}
