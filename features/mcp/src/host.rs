//! MCP on an [`EngineHost`]: `.mcp(…)` and `.mcp_at(…)`.
//!
//! architect cannot depend on this crate (this crate depends on it), so
//! the builder methods are an extension trait, [`McpHost`]. Bring it into
//! scope and an engine host serves MCP's Streamable HTTP transport beside
//! `/vox`:
//!
//! ```ignore
//! use architect_mcp::host::{McpHost, McpTarget};
//!
//! EngineHost::new(router, bind)
//!     // Per org, served for the caller the bearer names.
//!     .mcp_at("/org/{slug}/mcp", move |head: RequestHead| {
//!         let orgs = orgs.clone();
//!         async move {
//!             let org = orgs.get(head.param("slug").unwrap_or_default())
//!                 .ok_or(StatusCode::NOT_FOUND)?;
//!             let who = org.principal_for(head.bearer()).await
//!                 .ok_or(StatusCode::UNAUTHORIZED)?;
//!             Ok(McpTarget::new(org.mcp.clone(), ActionCaller::new(who)))
//!         }
//!     })
//! ```

use std::future::Future;
use std::sync::Arc;

use architect::action::ActionCaller;
use architect::host::{EngineHost, RequestHead};
use axum::body::Bytes;
use axum::response::IntoResponse;
use axum::routing::post;

use crate::Server;

/// What an [`McpHost::mcp_at`] resolver answers: the server a request is
/// for, and who it runs as.
#[derive(Debug, Clone)]
pub struct McpTarget {
    /// The server (its Action List, tools and instructions).
    pub server: Arc<Server>,
    /// Who the request runs as: its principal, checked by the list's
    /// authorizer, and the context its actions' handlers read.
    /// [`ActionCaller::app`] runs it as the server's own principal.
    pub caller: ActionCaller,
}

impl McpTarget {
    /// `server`, run as `caller`.
    #[must_use]
    pub const fn new(server: Arc<Server>, caller: ActionCaller) -> Self {
        Self { server, caller }
    }
}

/// MCP builder methods on [`EngineHost`].
pub trait McpHost: Sized {
    /// `POST /mcp` serving `server`, guarded as [`crate::http::router`]
    /// guards it: only a local `Origin`, and the bearer `token` when there
    /// is one. For an app driven by a model on the same machine.
    #[must_use]
    fn mcp(self, server: Server, token: Option<String>) -> Self;

    /// `POST path` (`/org/{slug}/mcp`, `/mcp`), for a server on the
    /// network. Each request calls `resolve` with its [`RequestHead`]
    /// (path params, headers, [`RequestHead::bearer`]) first. It answers
    /// with the [`McpTarget`] to serve, or anything [`IntoResponse`] to
    /// refuse (`401`, `404`). There is no origin check: authentication is
    /// the resolver's.
    #[must_use]
    fn mcp_at<E, F, Fut>(self, path: &str, resolve: F) -> Self
    where
        F: Fn(RequestHead) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<McpTarget, E>> + Send + 'static,
        E: IntoResponse;
}

impl McpHost for EngineHost {
    fn mcp(self, server: Server, token: Option<String>) -> Self {
        self.plugin(crate::http::router(server, token))
    }

    fn mcp_at<E, F, Fut>(self, path: &str, resolve: F) -> Self
    where
        F: Fn(RequestHead) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<McpTarget, E>> + Send + 'static,
        E: IntoResponse,
    {
        let messages = post(move |head: RequestHead, body: Bytes| {
            let resolved = resolve(head);
            async move {
                // Bound first, so the refusal (`E`, not `Send`) is gone
                // before the answer is awaited.
                let target = match resolved.await {
                    Ok(target) => target,
                    Err(refused) => return refused.into_response(),
                };
                crate::http::answer(target.server, target.caller, &body).await
            }
        });
        self.plugin(crate::http::mount(path, messages))
    }
}
