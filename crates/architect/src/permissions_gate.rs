//! The router-level permission gate — architect's enforcement point.
//!
//! Wraps a [`LayerRouter`](crate::layer::LayerRouter) so every inbound call
//! is checked against an [`architect_permissions::PermissionEngine`] BEFORE
//! dispatch. vox's `ServerMiddleware` can observe requests but cannot refuse
//! them; a wrapping [`vox::Handler`] can — it owns the reply sink, so a denied
//! call is answered with an error and the inner handler never runs.
//!
//! Granularity: METHOD-level. Each service registers a
//! [`architect_permissions::ServicePermits`] table mapping method →
//! (action, resource template);
//! the gate checks the template's `coarse_resource`
//! (`vault/{path}` → `vault/**`). Argument-level distinctions (the exact
//! `{path}`) are the service impl's job via a direct
//! [`architect_permissions::PermissionEngine::check`] — same engine, finer
//! resource. Methods
//! missing from a registered table are DENIED (fail-closed); services with
//! no table follow the gate's `UnlistedPolicy`.
//!
//! Deny wire form: `VoxError::InvalidPayload("permission denied: …")`,
//! encoded in the METHOD'S OWN response wire shape
//! (`Result<T, VoxError<E>>`, built reflectively from
//! `response_wire_shape(method_id)`) so the client decodes the reason
//! verbatim — vox 0.10 has no dedicated forbidden variant, and `User(E)`
//! is method-typed. If reflective construction ever fails the gate falls
//! back to the type-erased `send_error` (the call still fails; the reason
//! may arrive schema-mangled). The server audit log always carries it.
//!
//! Design: `apps/task/plans/architect-permissions.md`.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use architect_permissions::{
    Action, AuditEvent, AuditSink, Decision, IdentityResolver, MethodPermit, PermissionEngine,
    Resource, ServicePermits,
};
use vox::{
    DriverReplySink, Handler, MethodId, RequestCall, SchemaRecvTracker, SelfRef, ServiceDescriptor,
};

use crate::layer::LayerRouter;

use architect_permissions::Principal;
#[cfg(feature = "telemetry")]
use tracing::Instrument as _;

/// Metadata key carrying `Bearer <token>` (mirrors auth-proto's
/// `AUTHORIZATION_METADATA_KEY`; duplicated here so architect does not
/// depend on auth-proto).
pub const AUTHORIZATION_METADATA_KEY: &str = "authorization";

/// What to do with calls to services that registered NO permit table.
///
/// The default is [`Deny`](UnlistedPolicy::Deny). It used to be `Allow`,
/// on the reasoning that permit tables could then arrive service by
/// service without breaking the rest — but a gate that serves untabled
/// services *silently* is not a migration mode, it is the absence of a
/// gate wearing one's clothes. In one deployment 68 of 70 mounted
/// services sat unchecked behind it for months, and nothing said so.
///
/// The migration mode is [`observe_only`](PermissionsGate::observe_only):
/// it evaluates and audits every decision while refusing nothing, so the
/// audit log tells you exactly what enforcement *would* have done before
/// you switch it on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum UnlistedPolicy {
    /// Let them through unchecked.
    ///
    /// Use this only with a deliberate, dated reason. Prefer
    /// [`observe_only`](PermissionsGate::observe_only) for a rollout and
    /// [`coverage`](PermissionsGate::coverage) to see what is missing.
    Allow,
    /// Refuse them. The default: a service nobody wrote a table for is
    /// not a service anybody decided to expose.
    #[default]
    Deny,
}

#[derive(Clone)]
struct GateRule {
    service: &'static str,
    method: &'static str,
    action: Action,
    coarse: Resource,
    audit_allow: bool,
}

/// Builder + runtime state for a permissioned router.
#[derive(Clone)]
pub struct PermissionsGate {
    engine: Arc<dyn PermissionEngine>,
    identity: Arc<dyn IdentityResolver>,
    audit: Arc<dyn AuditSink>,
    rules: HashMap<MethodId, GateRule>,
    /// Method ids that belong to services WITH a table but are unlisted →
    /// explicit deny.
    tabled_unlisted: HashMap<MethodId, &'static str>,
    /// Every `(service, method)` a registered table NAMED, whether or not
    /// the descriptor actually has that method. Feeds
    /// [`coverage`](Self::coverage)'s phantom check.
    ///
    /// Keyed by the **descriptor's** `service_name`, not the table's
    /// `service` label: the label is free-form text for audit lines
    /// (`"gate-probe"` for a `GateProbe` service), while the descriptor
    /// name is what the router mounts under. Coverage cross-references
    /// the router, so it has to speak the router's names.
    declared: BTreeSet<(&'static str, &'static str)>,
    unlisted: UnlistedPolicy,
    /// Observe-only mode: run every check + audit, but never refuse. The
    /// rollout switch — flip off once the audit log runs clean.
    observe_only: bool,
}

impl PermissionsGate {
    pub fn new(engine: Arc<dyn PermissionEngine>, identity: Arc<dyn IdentityResolver>) -> Self {
        Self {
            engine,
            identity,
            audit: Arc::new(architect_permissions::TracingAudit),
            rules: HashMap::new(),
            tabled_unlisted: HashMap::new(),
            declared: BTreeSet::new(),
            unlisted: UnlistedPolicy::Deny,
            observe_only: false,
        }
    }

    #[must_use]
    pub fn with_audit(mut self, audit: Arc<dyn AuditSink>) -> Self {
        self.audit = audit;
        self
    }

    #[must_use]
    pub const fn unlisted(mut self, policy: UnlistedPolicy) -> Self {
        self.unlisted = policy;
        self
    }

    /// Observe-only: evaluate + audit every decision but enforce nothing.
    #[must_use]
    pub const fn observe_only(mut self, on: bool) -> Self {
        self.observe_only = on;
        self
    }

    /// Register a service's permit table. Table methods resolve to method
    /// ids through `descriptor`; descriptor methods NOT in the table are
    /// recorded as explicit denies (fail-closed).
    #[must_use]
    pub fn permit(
        mut self,
        descriptor: &'static ServiceDescriptor,
        table: &ServicePermits,
    ) -> Self {
        for permit in table.methods {
            self.declared
                .insert((descriptor.service_name, permit.method));
        }
        for method in descriptor.methods {
            let permit: Option<&MethodPermit> = table
                .methods
                .iter()
                .find(|p| p.method == method.method_name);
            match permit {
                Some(p) => {
                    self.rules.insert(
                        method.id,
                        GateRule {
                            service: table.service,
                            method: p.method,
                            action: Action::new(p.action),
                            coarse: p.coarse_resource(),
                            audit_allow: p.audit,
                        },
                    );
                }
                None => {
                    self.tabled_unlisted.insert(method.id, table.service);
                }
            }
        }
        self
    }

    /// What this gate actually covers of `router`'s mounted surface.
    ///
    /// The gate only knows the tables it was handed; the router knows what
    /// is mounted. Neither half can spot a gap alone, which is how a
    /// deployment ends up serving dozens of untabled services without
    /// anything saying so. Cross-referencing them is the whole job.
    #[must_use]
    pub fn coverage(&self, router: &LayerRouter) -> GateCoverage {
        let mounted = router.mounted();
        let tabled: BTreeSet<&'static str> =
            self.declared.iter().map(|(service, _)| *service).collect();

        let mut methods = 0usize;
        let mut permitted = 0usize;
        let mut untabled = Vec::new();
        let mut uncovered = Vec::new();
        let mut real: BTreeSet<(&'static str, &'static str)> = BTreeSet::new();

        for (service, service_methods) in &mounted {
            methods = methods.saturating_add(service_methods.len());
            if !tabled.contains(service) {
                untabled.push(*service);
                continue;
            }
            for method in service_methods {
                real.insert((*service, *method));
                if self.declared.contains(&(*service, *method)) {
                    permitted = permitted.saturating_add(1);
                } else {
                    uncovered.push((*service, *method));
                }
            }
        }

        // Named by a table but absent from the descriptor: a typo or a
        // renamed method. The permit is dead, and the real method — if
        // there is one — is silently fail-closed.
        let phantom: Vec<(&'static str, &'static str)> = self
            .declared
            .iter()
            .filter(|entry| mounted.contains_key(entry.0) && !real.contains(*entry))
            .copied()
            .collect();

        GateCoverage {
            services: mounted.len(),
            tabled: mounted.keys().filter(|s| tabled.contains(*s)).count(),
            untabled,
            methods,
            permitted,
            uncovered,
            phantom,
            unlisted: self.unlisted,
            observe_only: self.observe_only,
        }
    }

    /// Wrap a router with this gate.
    ///
    /// Reports [`coverage`](Self::coverage) through `tracing` on the way
    /// past — one `info` line always, plus a `warn` per gap. Every app
    /// that mounts a gate gets the blind-spot report for free; nobody has
    /// to remember to ask for it.
    #[must_use]
    pub fn wrap(self, inner: LayerRouter) -> PermissionedRouter {
        self.coverage(&inner).report();
        self.wrap_handler(inner)
    }

    /// Wrap ANY handler (router or router-wrapper) with this gate.
    ///
    /// Cannot report coverage — the handler is already type-erased. Call
    /// [`GateCoverage::report`] yourself if you have the router.
    pub fn wrap_handler<H>(self, inner: H) -> PermissionedRouter<H> {
        PermissionedRouter {
            inner,
            gate: Arc::new(self),
            connection_bearer: None,
        }
    }

    /// Wrap with an ALREADY-SHARED gate (one gate, many lanes/connections —
    /// the per-connection serve path).
    pub const fn wrap_shared<H>(gate: Arc<Self>, inner: H) -> PermissionedRouter<H> {
        PermissionedRouter {
            inner,
            gate,
            connection_bearer: None,
        }
    }

    /// [`wrap_shared`](Self::wrap_shared) with a **connection-scoped**
    /// bearer: the identity presented ONCE at transport establish (a
    /// WebSocket upgrade header or subprotocol) rather than on each call.
    ///
    /// Browsers cannot set arbitrary WebSocket headers and must not put a
    /// token in the URL (it lands in proxy + access logs), so the token
    /// rides the handshake and applies to every call on that connection.
    /// Per-call `authorization` metadata still WINS where present, so a
    /// per-typed-client [`ClientMiddleware`] and this can coexist — a
    /// connection carrying one identity can still make a call as another.
    ///
    /// [`ClientMiddleware`]: vox::ClientMiddleware
    pub fn wrap_shared_with_bearer<H>(
        gate: Arc<Self>,
        inner: H,
        bearer: Option<String>,
    ) -> PermissionedRouter<H> {
        PermissionedRouter {
            inner,
            gate,
            connection_bearer: bearer.map(Arc::from),
        }
    }

    /// The engine this gate consults — for mounting a `PermissionsService`
    /// oracle that can never disagree with enforcement.
    #[must_use]
    pub fn engine(&self) -> Arc<dyn PermissionEngine> {
        self.engine.clone()
    }

    /// The identity resolver this gate uses.
    #[must_use]
    pub fn identity_resolver(&self) -> Arc<dyn IdentityResolver> {
        self.identity.clone()
    }

    fn bearer_from(metadata: &vox::Metadata) -> Option<String> {
        use vox::MetadataExt;
        metadata
            .meta_str(AUTHORIZATION_METADATA_KEY)
            .and_then(|v| v.strip_prefix("Bearer "))
            .map(|s| s.trim().to_string())
    }

    /// Decide from OWNED request facts (method id + bearer token) — the
    /// `RequestCall` borrow is not `Sync` and must not cross an await.
    async fn decide(&self, method_id: MethodId, token: Option<String>) -> GateOutcome {
        if let Some(rule) = self.rules.get(&method_id) {
            let who = self.identity.resolve(token.as_deref()).await;
            let decision = self.engine.check(&who, &rule.coarse, &rule.action);
            let allowed = decision.allowed();
            if !allowed || rule.audit_allow {
                self.audit.record(AuditEvent {
                    principal: who.describe(),
                    resource: rule.coarse.as_str().to_string(),
                    action: rule.action.as_str().to_string(),
                    allowed,
                    reason: match &decision {
                        Decision::Deny { reason } => Some(reason.clone()),
                        Decision::Allow => None,
                    },
                });
            }
            match decision {
                Decision::Allow => GateOutcome::Pass(who),
                Decision::Deny { reason } => GateOutcome::Deny(format!(
                    "permission denied: {reason} ({}/{})",
                    rule.service, rule.method
                )),
            }
        } else if let Some(service) = self.tabled_unlisted.get(&method_id) {
            let who = self.identity.resolve(token.as_deref()).await.describe();
            self.audit.record(AuditEvent {
                principal: who,
                resource: format!("service/{service}"),
                action: "call".into(),
                allowed: false,
                reason: Some("method not in permit table (fail-closed)".into()),
            });
            GateOutcome::Deny(format!(
                "permission denied: {service} method not permitted on this lane"
            ))
        } else {
            match self.unlisted {
                // Unlisted-but-allowed still resolves, so a handler on an
                // ungated method sees the same identity it would on a
                // gated one. Skipping the resolve here would make
                // `caller()` depend on whether a permit row happens to
                // exist, which is not a distinction a method can reason
                // about.
                UnlistedPolicy::Allow => {
                    GateOutcome::Pass(self.identity.resolve(token.as_deref()).await)
                }
                UnlistedPolicy::Deny => {
                    let who = self.identity.resolve(token.as_deref()).await.describe();
                    self.audit.record(AuditEvent {
                        principal: who,
                        resource: format!("method/{method_id:?}"),
                        action: "call".into(),
                        allowed: false,
                        reason: Some("service not mounted for this lane".into()),
                    });
                    GateOutcome::Deny(
                        "permission denied: service not available on this lane".into(),
                    )
                }
            }
        }
    }
}

enum GateOutcome {
    /// Allowed, and by whom. The principal rides along because the gate
    /// is the only place on the request path that resolves one, and a
    /// handler that needs to know *who* asked would otherwise have to
    /// resolve it a second time — from a token it cannot see, since the
    /// metadata borrow does not outlive dispatch.
    Pass(Principal),
    Deny(String),
}

#[cfg(not(target_arch = "wasm32"))]
tokio::task_local! {
    /// The principal the gate resolved for the call running on this task.
    static CALLER: Principal;
}

/// WHO is asking, inside a handler the gate dispatched.
///
/// `None` when nothing resolved a principal for this call: a transport
/// with no gate in front of it, a handler reached some other way, or code
/// running off the request task. A caller-sensitive method must treat
/// that as "no identity" and refuse rather than fall back to a default —
/// the whole point of asking is that the answer differs per person, and a
/// default that happens to be permissive is how an owner shortcut
/// silently becomes everyone's.
///
/// Set by [`PermissionedRouter`] around the inner handler, so it is
/// available for the whole of a method's execution and gone afterwards.
#[cfg(not(target_arch = "wasm32"))]
#[must_use]
pub fn caller() -> Option<Principal> {
    CALLER.try_with(Clone::clone).ok()
}

/// Any handler behind a [`PermissionsGate`] — a bare [`LayerRouter`] or an
/// already-wrapped one (e.g. a snapshot-gating router).
///
/// `Handler` for the same sinks, so it drops into every transport
/// (`axum_ws`, iroh, `LocalServer`) exactly where the inner handler would.
#[derive(Clone)]
pub struct PermissionedRouter<H = LayerRouter> {
    inner: H,
    gate: Arc<PermissionsGate>,
    /// Identity presented at transport establish, applied to every call on
    /// this connection when the call itself carries no `authorization`
    /// metadata. See [`PermissionsGate::wrap_shared_with_bearer`].
    connection_bearer: Option<Arc<str>>,
}

impl<H> PermissionedRouter<H> {
    pub const fn inner(&self) -> &H {
        &self.inner
    }
}

impl<H> Handler<DriverReplySink> for PermissionedRouter<H>
where
    H: Handler<DriverReplySink> + Send + Sync,
{
    fn args_have_channels(&self, method_id: MethodId) -> bool {
        self.inner.args_have_channels(method_id)
    }

    fn response_wire_shape(&self, method_id: MethodId) -> Option<&'static facet::Shape> {
        self.inner.response_wire_shape(method_id)
    }

    async fn handle(
        &self,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) {
        let (method_id, token) = {
            let c = call.get();
            (c.method_id, PermissionsGate::bearer_from(&c.metadata))
        };
        // Per-call metadata wins; the connection's establish-time bearer is
        // the fallback. Browsers can't set WebSocket headers per call, so
        // for the web client this IS the identity on every call.
        let token = token.or_else(|| self.connection_bearer.as_deref().map(str::to_owned));
        // Everything below runs inside ONE span, which is the call's wide
        // event. This matters for ordering: `decide` resolves the identity
        // and audits the permission decision BEFORE dispatching, so it runs
        // before `LayerRouter` opens its own `rpc` span. Without a span
        // opened here, anything the gate records lands on whatever span
        // happens to be current — in a WebSocket driver task, none — and
        // is silently dropped. That is not hypothetical: the auth/perm
        // fields were added, compiled, deployed, and recorded nothing,
        // and only a query against the exported spans caught it.
        //
        // `LayerRouter`'s span nests inside this one and carries the
        // rpc.service/method detail.
        #[cfg(feature = "telemetry")]
        {
            // Named for the method, not a generic "rpc". This span is the
            // trace ROOT, so its name is what a trace list shows — leaving
            // it generic collapses every call in the UI into one
            // indistinguishable bucket, which is the same readability
            // failure the per-method naming in `LayerRouter` exists to
            // avoid. The permit table already knows the names.
            let name = self.gate.rules.get(&method_id).map_or_else(
                || "rpc".to_owned(),
                |r| format!("{}/{}", r.service, r.method),
            );
            self.dispatch(method_id, token, call, reply, schemas)
                .instrument(tracing::info_span!("rpc.gated", otel.name = name))
                .await;
        }
        // Without the feature there is no span to open and no `tracing`
        // dependency to open it with — dispatch directly.
        #[cfg(not(feature = "telemetry"))]
        self.dispatch(method_id, token, call, reply, schemas).await;
    }
}

impl<H> PermissionedRouter<H>
where
    H: Handler<DriverReplySink> + Send + Sync,
{
    async fn dispatch(
        &self,
        method_id: MethodId,
        token: Option<String>,
        call: SelfRef<RequestCall<'static>>,
        reply: DriverReplySink,
        schemas: Arc<SchemaRecvTracker>,
    ) {
        // Kept for the observe-only arm below, which has to resolve the
        // same identity `decide` did.
        let presented = token.clone();
        let outcome = self.gate.decide(method_id, token).await;
        match outcome {
            GateOutcome::Pass(who) => {
                CALLER
                    .scope(who, self.inner.handle(call, reply, schemas))
                    .await;
            }
            GateOutcome::Deny(reason) if self.gate.observe_only => {
                // Observe-only: the deny was already audited by decide();
                // note it and let the call through.
                self.gate.audit.record(AuditEvent {
                    principal: "observe-only".into(),
                    resource: "gate".into(),
                    action: "would-deny".into(),
                    allowed: true,
                    reason: Some(reason),
                });
                // Observe-only must not change what a handler sees
                // either: a method that reads `caller()` has to behave
                // the same whether enforcement is on, or turning it on
                // becomes a behaviour change rather than a refusal.
                let who = self.gate.identity.resolve(presented.as_deref()).await;
                CALLER
                    .scope(who, self.inner.handle(call, reply, schemas))
                    .await;
            }
            GateOutcome::Deny(reason) => {
                let shape = self.inner.response_wire_shape(method_id);
                deny_reply(reply, shape, reason).await;
            }
        }
    }
}

/// Reply a denial in the METHOD'S OWN response wire shape so the client
/// decodes `VoxError::InvalidPayload(reason)` cleanly. Falls back to the
/// type-erased `send_error` when the shape is unknown or reflective
/// construction fails.
#[allow(clippy::default_trait_access)]
async fn deny_reply(reply: DriverReplySink, shape: Option<&'static facet::Shape>, reason: String) {
    use vox::ReplySink as _;
    // Keep the built value alive across the send. `HeapValue` is
    // conservatively `!Send` (raw pointers); the built response is plain
    // owned wire data with no thread affinity.
    struct SendValue(vox::facet_reflect::HeapValue<'static, false>);
    // SAFETY: `HeapValue` is `!Send` only because it holds raw pointers.
    // The value inside is plain owned wire data built here from a
    // `facet::Shape` — no thread affinity, no borrowed interior. It is
    // moved into this wrapper immediately and dropped on the same task.
    #[allow(clippy::non_send_fields_in_send_ty)]
    unsafe impl Send for SendValue {}
    if let Some(shape) = shape {
        // Map into the Send wrapper IMMEDIATELY so no `!Send` binding can
        // live across the await below.
        let built = build_denied_response(shape, &reason).map(SendValue);
        if let Some(holder) = built {
            // SAFETY: the pointer targets `holder`'s live `HeapValue` of
            // exactly `shape`; `holder` outlives the send below.
            let ret = {
                let ptr = holder.0.peek().data();
                unsafe { vox::Payload::outgoing_unchecked(ptr, shape) }
            };
            reply
                .send_reply(vox::RequestResponse {
                    ret,
                    // `Default::default()`, not the named types: `SchemaBytes`
                    // lives in `vox-types`, which is an OPTIONAL dependency
                    // (feature `local`) — naming it here would make this
                    // module fail to build in the `vox`-only configuration.
                    metadata: Default::default(),
                    schemas: Default::default(),
                })
                .await;
            drop(holder);
            return;
        }
        tracing_warn_fallback(&reason);
    }
    reply
        .send_error(vox::VoxError::<core::convert::Infallible>::InvalidPayload(
            reason,
        ))
        .await;
}

fn tracing_warn_fallback(reason: &str) {
    // Kept out of the async fn so the gate has zero tracing spans on the
    // happy path.
    #[cfg(debug_assertions)]
    eprintln!(
        "permissions gate: shaped deny construction failed, sending type-erased deny ({reason})"
    );
    let _ = reason;
}

/// Build `Err(VoxError::InvalidPayload(reason))` as a value of the method's
/// `Result<T, VoxError<E>>` response shape, reflectively.
fn build_denied_response(
    shape: &'static facet::Shape,
    reason: &str,
) -> Option<vox::facet_reflect::HeapValue<'static, false>> {
    use vox::facet_reflect::{Partial, TypePlanCore};
    // SAFETY: `shape` is a real `'static` response shape provided by the
    // generated dispatcher (`response_wire_shape`).
    let plan = unsafe { TypePlanCore::from_shape(shape) }.ok()?;
    Partial::<false>::alloc_owned_with_plan(plan)
        .ok()?
        .begin_err()
        .ok()?
        .select_variant_named("InvalidPayload")
        .ok()?
        .set_nth_field(0, reason.to_string())
        .ok()?
        .end()
        .ok()?
        .build()
        .ok()
}

/// Convenience: gate helpers on [`LayerRouter`].
impl LayerRouter {
    /// Put this router behind a permissions gate.
    #[must_use]
    pub fn with_permissions(self, gate: PermissionsGate) -> PermissionedRouter {
        gate.wrap(self)
    }
}

/// Fixed-principal resolver re-export for share-lane construction.
pub use architect_permissions::StaticPrincipal;

/// What a [`PermissionsGate`] covers of a router's mounted surface.
///
/// Produced by [`PermissionsGate::coverage`] and reported automatically by
/// [`PermissionsGate::wrap`].
#[derive(Clone, Debug)]
pub struct GateCoverage {
    /// Services the router mounts.
    pub services: usize,
    /// …of which carry a permit table.
    pub tabled: usize,
    /// Mounted services with NO table — they follow
    /// [`UnlistedPolicy`](Self::unlisted).
    pub untabled: Vec<&'static str>,
    /// Methods across all mounted services.
    pub methods: usize,
    /// …of which a permit names.
    pub permitted: usize,
    /// `(service, method)` on the descriptor but missing from its table.
    /// **Fail-closed**: a tabled service denies everything it didn't list.
    pub uncovered: Vec<(&'static str, &'static str)>,
    /// `(service, method)` named by a table but absent from the
    /// descriptor — a typo or a rename. The permit is dead, and the real
    /// method is silently denied.
    pub phantom: Vec<(&'static str, &'static str)>,
    /// The gate's policy for [`untabled`](Self::untabled) services.
    pub unlisted: UnlistedPolicy,
    /// Whether the gate is evaluating without enforcing.
    pub observe_only: bool,
}

impl GateCoverage {
    /// Every mounted method resolves to a permit, and no permit is dead.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.untabled.is_empty() && self.uncovered.is_empty() && self.phantom.is_empty()
    }

    /// Emit the report: one `info` line, plus a `warn` per gap.
    ///
    /// Called by [`PermissionsGate::wrap`]. Call it yourself after
    /// [`PermissionsGate::wrap_shared`], which takes an already-erased
    /// handler and so cannot see the router.
    pub fn report(&self) {
        tracing::info!(
            services = self.services,
            tabled = self.tabled,
            methods = self.methods,
            permitted = self.permitted,
            unlisted = ?self.unlisted,
            observe_only = self.observe_only,
            "permissions gate: {}/{} services tabled ({}/{} methods)",
            self.tabled,
            self.services,
            self.permitted,
            self.methods,
        );
        if !self.untabled.is_empty() {
            let verdict = match self.unlisted {
                UnlistedPolicy::Allow => "SERVED UNCHECKED",
                UnlistedPolicy::Deny => "denied",
            };
            tracing::warn!(
                count = self.untabled.len(),
                services = %self.untabled.join(", "),
                "permissions gate: {} service(s) have NO permit table — {verdict}",
                self.untabled.len(),
            );
        }
        if !self.uncovered.is_empty() {
            tracing::warn!(
                count = self.uncovered.len(),
                methods = %join_pairs(&self.uncovered),
                "permissions gate: {} method(s) of a tabled service are unlisted — denied",
                self.uncovered.len(),
            );
        }
        if !self.phantom.is_empty() {
            tracing::warn!(
                count = self.phantom.len(),
                methods = %join_pairs(&self.phantom),
                "permissions gate: {} permit(s) name a method that does not exist — \
                 dead permit, and the real method (if any) is denied",
                self.phantom.len(),
            );
        }
    }
}

fn join_pairs(pairs: &[(&'static str, &'static str)]) -> String {
    pairs
        .iter()
        .map(|(service, method)| format!("{service}.{method}"))
        .collect::<Vec<_>>()
        .join(", ")
}
