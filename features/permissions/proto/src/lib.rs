//! Wire types + the `PermissionsService` RPC surface for
//! `architect-permissions`.
//!
//! The service is a per-lane affordance oracle: it answers "may I?" and
//! enumerates the caller's capability manifest so UIs render (and grey out)
//! affordances from data instead of discovering `PermissionDenied` errors.
//! It is mounted with the SAME engine + identity resolver the lane's gate
//! uses (`architect::permissions_gate`), so the oracle and the enforcement
//! can never disagree.
//!
//! Design: `apps/task/plans/architect-permissions.md`. A `#[subscribe]`
//! manifest stream (live retraction on permission flips) is planned to ride
//! the same trait once share mutations exist to publish from — additive.

use architect_permissions::{Action, PermissionEngine, Principal, Resource};

/// One `(resource pattern, actions)` affordance the caller holds.
#[derive(Clone, Debug, PartialEq, ::facet::Facet)]
pub struct CapabilityRule {
    pub resource: String,
    pub actions: Vec<String>,
}

/// The caller's affordance set for this lane.
#[derive(Clone, Debug, PartialEq, ::facet::Facet)]
pub struct CapabilityManifest {
    /// Who the lane believes the caller is (`user:<id>`, `guest:<link>`,
    /// `service:<name>`, `anonymous`) — display form only.
    pub principal: String,
    /// Affordances, engine order. Empty means "nothing enumerable", which
    /// UIs must treat as unknown (fall back to trying), not as no-access.
    pub rules: Vec<CapabilityRule>,
}

/// Errors from the permissions surface.
#[derive(Clone, Debug, PartialEq, ::facet::Facet, thiserror::Error)]
#[repr(u8)]
pub enum PermissionsError {
    #[error("permissions unavailable: {0}")]
    Unavailable(String),
}

#[architect::rpc]
pub trait PermissionsService {
    /// One-shot check: may the CALLER (as resolved by the lane's identity)
    /// perform `action` on `resource`?
    async fn can(&self, resource: String, action: String) -> Result<bool, PermissionsError>;

    /// The caller's capability manifest under `prefix` (empty prefix = all).
    async fn capabilities(
        &self,
        token: String,
        prefix: String,
    ) -> Result<CapabilityManifest, PermissionsError>;
}

/// The service impl: an engine + identity resolver pair (the same pair the
/// lane's gate runs). `Clone` is cheap — both halves ride `Arc`s (the rpc
/// dispatcher clones the backend per call).
pub struct Permissions<E, I> {
    engine: std::sync::Arc<E>,
    identity: std::sync::Arc<I>,
}

impl<E, I> Clone for Permissions<E, I> {
    fn clone(&self) -> Self {
        Self {
            engine: self.engine.clone(),
            identity: self.identity.clone(),
        }
    }
}

impl<E, I> Permissions<E, I>
where
    E: PermissionEngine,
    I: architect_permissions::IdentityResolver,
{
    pub fn new(engine: E, identity: I) -> Self {
        Self {
            engine: std::sync::Arc::new(engine),
            identity: std::sync::Arc::new(identity),
        }
    }

    async fn principal(&self, token: &str) -> Principal {
        let tok = (!token.is_empty()).then_some(token);
        self.identity.resolve(tok).await
    }
}

impl<E, I> PermissionsService for Permissions<E, I>
where
    E: PermissionEngine + 'static,
    I: architect_permissions::IdentityResolver + 'static,
{
    async fn can(&self, resource: String, action: String) -> Result<bool, PermissionsError> {
        // Anonymous-principal check: `can` is advisory; callers that hold a
        // token should use `capabilities` (which resolves it). The gate is
        // the authority either way.
        let who = Principal::Anonymous;
        Ok(self
            .engine
            .check(&who, &Resource::new(resource), &Action::new(action))
            .allowed())
    }

    async fn capabilities(
        &self,
        token: String,
        prefix: String,
    ) -> Result<CapabilityManifest, PermissionsError> {
        let who = self.principal(&token).await;
        let rules = self
            .engine
            .survey(&who, &Resource::new(prefix))
            .into_iter()
            .map(|(r, actions)| CapabilityRule {
                resource: r.0,
                actions: actions.into_iter().map(|a| a.0).collect(),
            })
            .collect();
        Ok(CapabilityManifest {
            principal: who.describe(),
            rules,
        })
    }
}
