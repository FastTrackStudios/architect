//! A vault share link's live documents, read-only (#847, part 3).
//!
//! A link's guest follows a live document of a kind (a `.session`: its
//! chart, cue list and transport) on `/s/<code>/_live`, a vox WebSocket
//! serving `DocSync` and `DocPresence`. It is read-only throughout:
//!
//! - **Who:** every call on the lane is the link's guest,
//!   `Principal::Guest { link_id: <code> }`, resolved by the lane's own
//!   gate and never by anything the caller sends.
//! - **Which documents:** before each session the link is opened again,
//!   as a page or file of it is ([`GuestLane::served`]: live, its audience
//!   admits the bearer), and the document's vault file must be one the
//!   link serves now: a note it lists, or a file at its layer that a
//!   listed note links to. Anything else is an unknown document.
//! - **What it may do:** the session runs through
//!   [`crdt::registry::DocRegistry::sync_read_only`] and
//!   `presence_read_only`. It follows every edit and sees everyone's
//!   presence, and the first update it sends ends its session; what it
//!   publishes reaches nobody. The registry admits a share guest only for
//!   a document its lane has granted ([`GuestGrants`]), and never for a
//!   write.
//! - **For how long:** while the link serves the document. It is checked
//!   again every [`RECHECK`], and the session ends when the link is
//!   revoked or expires, or the document leaves its scope.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use architect::host::{EngineHost, RequestHead};
use architect::http::axum::http::StatusCode;
use architect::layer::LayerRouter;
use architect::permissions_gate::{PermissionsGate, StaticPrincipal, UnlistedPolicy};
use architect_permissions::{AllowAll, Principal};
use crdt::registry::DocRegistry;
use crdt::sync::{
    DocPresence, DocPresenceDispatcher, DocSync, DocSyncDispatcher, SyncDown, SyncError,
    doc_presence_service_descriptor, doc_sync_service_descriptor,
};
use uuid::Uuid;
use vault_collab::VaultCollab;

use super::shares::GuestLane;

/// How often a live session asks again whether its link still serves
/// the document.
pub const RECHECK: Duration = Duration::from_secs(5);

/// The documents share guests are following now, by link code: what the
/// registry's authorizer admits a share guest for, and only while its
/// session lasts.
#[derive(Clone, Default)]
pub struct GuestGrants(Arc<Mutex<HashMap<(String, Uuid), usize>>>);

impl GuestGrants {
    /// Whether `who` is a share guest following `doc_id` through its
    /// link's lane now.
    #[must_use]
    pub fn holds(&self, who: Option<&Principal>, doc_id: Uuid) -> bool {
        let Some(Principal::Guest { link_id, .. }) = who else {
            return false;
        };
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&(link_id.clone(), doc_id))
    }

    /// Link `code`'s guest follows `doc_id` until the grant is dropped.
    fn grant(&self, code: &str, doc_id: Uuid) -> Grant {
        let key = (code.to_owned(), doc_id);
        *self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key.clone())
            .or_default() += 1;
        Grant {
            grants: self.clone(),
            key,
        }
    }
}

/// One session's grant: gone with the session.
struct Grant {
    grants: GuestGrants,
    key: (String, Uuid),
}

impl Drop for Grant {
    fn drop(&mut self) {
        let mut held = self.grants.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(n) = held.get_mut(&self.key) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                held.remove(&self.key);
            }
        }
    }
}

/// One guest connection's live documents, for link `code` and the
/// bearer it opened with.
#[derive(Clone)]
pub struct GuestLive {
    lane: GuestLane,
    registry: DocRegistry,
    collab: VaultCollab,
    grants: GuestGrants,
    code: String,
    token: String,
    recheck: Duration,
}

impl GuestLive {
    /// Whether the link serves the document `doc_id` to this guest now.
    async fn serves(&self, doc_id: Uuid) -> bool {
        let Ok((vault, served)) = self.lane.served(&self.code, self.token.clone()).await else {
            return false;
        };
        self.collab
            .document_route(doc_id)
            .is_some_and(|(v, path)| v == vault && served.contains(&path))
    }

    /// The grant for a session on `doc_id`, if the link serves it.
    async fn admit(&self, doc_id: Uuid) -> Result<Grant, SyncError> {
        if self.serves(doc_id).await {
            Ok(self.grants.grant(&self.code, doc_id))
        } else {
            tracing::warn!(
                share.live = "refused",
                doc = %doc_id,
                "share guest: a live document the link does not serve"
            );
            Err(SyncError::UnknownDoc)
        }
    }

    /// Returns once the link no longer serves `doc_id`.
    async fn until_closed(&self, doc_id: Uuid) {
        loop {
            tokio::time::sleep(self.recheck).await;
            if !self.serves(doc_id).await {
                return;
            }
        }
    }
}

impl DocSync for GuestLive {
    async fn sync(
        &self,
        doc_id: Uuid,
        from: Vec<u8>,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<SyncDown>,
    ) -> Result<(), SyncError> {
        let _grant = self.admit(doc_id).await?;
        tokio::select! {
            ended = self.registry.sync_read_only(doc_id, from, up, down) => ended,
            () = self.until_closed(doc_id) => Err(SyncError::UnknownDoc),
        }
    }
}

impl DocPresence for GuestLive {
    async fn presence(
        &self,
        doc_id: Uuid,
        up: vox::Rx<Vec<u8>>,
        down: vox::Tx<Vec<u8>>,
    ) -> Result<(), SyncError> {
        let _grant = self.admit(doc_id).await?;
        tokio::select! {
            ended = self.registry.presence_read_only(doc_id, up, down) => ended,
            () = self.until_closed(doc_id) => Err(SyncError::UnknownDoc),
        }
    }

    async fn now(&self) -> f64 {
        crdt::presence::clock_ms()
    }
}

/// Serves share links' live documents at `/s/{code}/_live` on `host`.
///
/// The upgrade itself is refused unless link `code` is live for the
/// bearer (an issuer session for a link for signed-in people; none
/// otherwise), as a request for its page would be.
#[must_use]
pub fn mount(
    host: EngineHost,
    lane: GuestLane,
    registry: DocRegistry,
    collab: VaultCollab,
    grants: GuestGrants,
) -> EngineHost {
    host.vox_at_permissioned("/s/{code}/_live", move |head: RequestHead| {
        let (lane, registry, collab, grants) = (
            lane.clone(),
            registry.clone(),
            collab.clone(),
            grants.clone(),
        );
        async move {
            let code = head.param("code").unwrap_or_default().to_owned();
            let bearer = head.bearer().map(str::to_owned);
            let token = bearer.clone().unwrap_or_default();
            if lane.served(&code, token.clone()).await.is_err() {
                return Err(StatusCode::NOT_FOUND);
            }
            let guest = Principal::Guest {
                link_id: code.clone(),
                display: None,
            };
            let gate = PermissionsGate::new(Arc::new(AllowAll), Arc::new(StaticPrincipal(guest)))
                .unlisted(UnlistedPolicy::Allow);
            let live = GuestLive {
                lane,
                registry,
                collab,
                grants,
                code,
                token,
                recheck: RECHECK,
            };
            let router = LayerRouter::new()
                .with(
                    doc_sync_service_descriptor(),
                    DocSyncDispatcher::new(live.clone()),
                )
                .with(
                    doc_presence_service_descriptor(),
                    DocPresenceDispatcher::new(live),
                );
            Ok(PermissionsGate::wrap_shared_with_bearer(
                Arc::new(gate),
                router,
                bearer,
            ))
        }
    })
}
