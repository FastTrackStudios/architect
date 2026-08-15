//! Framework-level authorization for architect services.
//!
//! One question, asked everywhere: **may `Principal` perform `Action` on
//! `Resource`?** Engines answer it; the enforcement points live elsewhere
//! (`architect::permissions_gate` wraps a `LayerRouter`, service impls call
//! [`PermissionEngine::check`] directly for arg-level decisions).
//!
//! Design: `apps/task/plans/architect-permissions.md`. First consumers: the
//! Task org lane (role-based) and the collaboration share lane (scope-based,
//! `apps/task/plans/collaboration-sharing.md`).
//!
//! The crate is deliberately dependency-light (no architect, no vox) so the
//! framework, auth, and product crates can all depend on it without cycles.
//! Wire/service types live in `architect-permissions-proto`.

use std::sync::Arc;

// ─── Send bounds ─────────────────────────────────────────────────────────
// Mirrors architect's MaybeSend seam: native engines cross threads (tokio
// multi-thread), wasm vox is single-threaded.

#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSendSync: Send + Sync {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> MaybeSendSync for T {}
#[cfg(target_arch = "wasm32")]
pub trait MaybeSendSync {}
#[cfg(target_arch = "wasm32")]
impl<T> MaybeSendSync for T {}

#[cfg(not(target_arch = "wasm32"))]
pub type BoxIdentityFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Principal> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
pub type BoxIdentityFuture<'a> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Principal> + 'a>>;

// ─── Core nouns ──────────────────────────────────────────────────────────

/// WHO is asking. Produced by an [`IdentityResolver`] (never by the caller).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Principal {
    /// A validated session → a known user.
    User { user_id: String },
    /// A share-lane visitor holding a live share-link token.
    Guest {
        link_id: String,
        display: Option<String>,
    },
    /// A trusted in-process / service caller.
    Service { name: String },
    /// No (valid) credential presented.
    Anonymous,
}

impl Principal {
    /// Short display form for audit lines and deny reasons.
    pub fn describe(&self) -> String {
        match self {
            Principal::User { user_id } => format!("user:{user_id}"),
            Principal::Guest { link_id, .. } => format!("guest:{link_id}"),
            Principal::Service { name } => format!("service:{name}"),
            Principal::Anonymous => "anonymous".to_string(),
        }
    }
}

/// WHAT is being touched. Hierarchical and path-shaped so allowlists are
/// cheap prefix/glob matches. Conventions (see the design doc):
/// `vault/<path>`, `media/<hash>`, `doc/<doc-id>`,
/// `threads/<entity_type>/<entity_id>`, `service/<name>/<method>`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Resource(pub String);

impl Resource {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Resource {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}
impl From<String> for Resource {
    fn from(s: String) -> Self {
        Self(s)
    }
}

/// The verb. A small fixed core plus free-form domain extensions
/// (`"download"`, `"invite"`, …).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Action(pub String);

impl Action {
    pub const READ: &'static str = "read";
    pub const WRITE: &'static str = "write";
    pub const COMMENT: &'static str = "comment";
    pub const ADMIN: &'static str = "admin";
    pub const DOWNLOAD: &'static str = "download";

    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
    pub fn read() -> Self {
        Self::new(Self::READ)
    }
    pub fn write() -> Self {
        Self::new(Self::WRITE)
    }
    pub fn comment() -> Self {
        Self::new(Self::COMMENT)
    }
    pub fn admin() -> Self {
        Self::new(Self::ADMIN)
    }
}

impl From<&str> for Action {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// The answer — with the WHY, so denials surface as actionable errors and
/// audit lines instead of bare `false`s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

impl Decision {
    pub fn deny(reason: impl Into<String>) -> Self {
        Decision::Deny {
            reason: reason.into(),
        }
    }
    pub fn allowed(&self) -> bool {
        matches!(self, Decision::Allow)
    }
}

// ─── The engine ──────────────────────────────────────────────────────────

/// May `who` perform `action` on `what`? Implementations are cheap,
/// synchronous, and read-only — anything needing I/O materializes its rule
/// set up front (see [`ScopeEngine`]) or caches (identity resolution is a
/// separate, async concern: [`IdentityResolver`]).
pub trait PermissionEngine: MaybeSendSync {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision;

    /// Bulk affordance query: every `(resource pattern, actions)` this
    /// principal holds under `prefix` — powers UI capability manifests.
    /// Engines that can't enumerate return an empty survey (callers must
    /// treat that as "unknown", not "nothing").
    fn survey(&self, _who: &Principal, _prefix: &Resource) -> Vec<(Resource, Vec<Action>)> {
        Vec::new()
    }
}

/// WHO is on the other end of this request. Async (token validation may hit
/// a session store); the gate resolves once per request and caches per
/// token where the implementation supports it.
pub trait IdentityResolver: MaybeSendSync {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a>;
}

impl<T: PermissionEngine + ?Sized> PermissionEngine for Arc<T> {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        (**self).check(who, what, action)
    }
    fn survey(&self, who: &Principal, prefix: &Resource) -> Vec<(Resource, Vec<Action>)> {
        (**self).survey(who, prefix)
    }
}

impl<T: IdentityResolver + ?Sized> IdentityResolver for Arc<T> {
    fn resolve<'a>(&'a self, bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        (**self).resolve(bearer_token)
    }
}

/// A fixed identity for every request — the share lane (every caller on a
/// share-token lane IS that link's guest) and tests.
#[derive(Clone, Debug)]
pub struct StaticPrincipal(pub Principal);

impl IdentityResolver for StaticPrincipal {
    fn resolve<'a>(&'a self, _bearer_token: Option<&'a str>) -> BoxIdentityFuture<'a> {
        let p = self.0.clone();
        Box::pin(async move { p })
    }
}

// ─── Glob matching ───────────────────────────────────────────────────────

/// Match `pattern` against a resource path. Supported syntax (segment
/// separator `/`): literal segments, `*` (one segment), `**` (any tail —
/// only meaningful as the final segment), and a trailing `/` prefix form
/// (`vault/Songs/` ≡ `vault/Songs/**`). An empty pattern matches nothing;
/// `**` alone matches everything.
pub fn glob_matches(pattern: &str, path: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    if let Some(prefix) = pattern.strip_suffix('/') {
        return path == prefix || path.starts_with(&format!("{prefix}/"));
    }
    let pat: Vec<&str> = pattern.split('/').collect();
    let segs: Vec<&str> = path.split('/').collect();
    glob_segments(&pat, &segs)
}

fn glob_segments(pat: &[&str], segs: &[&str]) -> bool {
    match (pat.first(), segs.first()) {
        (None, None) => true,
        (Some(&"**"), _) => {
            // `**` swallows any (possibly empty) tail; anything after it in
            // the pattern must match some suffix.
            if pat.len() == 1 {
                return true;
            }
            (0..=segs.len()).any(|skip| glob_segments(&pat[1..], &segs[skip..]))
        }
        (Some(&"*"), Some(_)) => glob_segments(&pat[1..], &segs[1..]),
        (Some(p), Some(s)) if p == s => glob_segments(&pat[1..], &segs[1..]),
        _ => false,
    }
}

// ─── Rules + engines ─────────────────────────────────────────────────────

/// One grant: these actions on resources matching this pattern.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Rule {
    pub resource: String,
    pub actions: Vec<String>,
}

impl Rule {
    pub fn new(resource: impl Into<String>, actions: &[&str]) -> Self {
        Self {
            resource: resource.into(),
            actions: actions.iter().map(|s| s.to_string()).collect(),
        }
    }

    pub fn permits(&self, what: &Resource, action: &Action) -> bool {
        self.actions.iter().any(|a| a == action.as_str() || a == "*")
            && glob_matches(&self.resource, what.as_str())
    }
}

/// A materialized allowlist for ONE principal shape — the share lane's
/// engine (built from an expanded `ShareScope`), also usable standalone in
/// tests. Checks ignore the principal: whoever reached this engine holds
/// exactly these rules.
#[derive(Clone, Debug, Default)]
pub struct ScopeEngine {
    rules: Vec<Rule>,
}

impl ScopeEngine {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }
}

impl PermissionEngine for ScopeEngine {
    fn check(&self, _who: &Principal, what: &Resource, action: &Action) -> Decision {
        if self.rules.iter().any(|r| r.permits(what, action)) {
            Decision::Allow
        } else {
            Decision::deny(format!(
                "outside scope: {} on {}",
                action.as_str(),
                what.as_str()
            ))
        }
    }

    fn survey(&self, _who: &Principal, prefix: &Resource) -> Vec<(Resource, Vec<Action>)> {
        self.rules
            .iter()
            .filter(|r| prefix.as_str().is_empty() || r.resource.starts_with(prefix.as_str()))
            .map(|r| {
                (
                    Resource::new(r.resource.clone()),
                    r.actions.iter().map(|a| Action::new(a.clone())).collect(),
                )
            })
            .collect()
    }
}

/// Role → rules, for org members. Role rule sets come from
/// `AuthOrganizationRole.permissions_json` (a JSON `[{resource, actions}]`
/// list) plus built-in defaults for `owner`/`member`/`guest`; the caller
/// supplies the `user_id → role` mapping (from `AuthMember`) at
/// construction/refresh time so checks stay synchronous.
#[derive(Clone, Debug, Default)]
pub struct RoleEngine {
    /// role name → rules.
    roles: std::collections::HashMap<String, Vec<Rule>>,
    /// user id → role name.
    members: std::collections::HashMap<String, String>,
    /// Services (in-process callers) bypass role lookup.
    allow_services: bool,
    /// Role assumed for a VALIDATED user with no membership row. `None`
    /// (default) denies unknown users. The Task org lane sets
    /// `Some("member")`: a user existing in the org's own auth store is a
    /// de-facto member until per-row membership sync lands with shares.
    default_user_role: Option<String>,
}

impl RoleEngine {
    pub fn new() -> Self {
        Self {
            roles: Self::default_roles(),
            members: Default::default(),
            allow_services: true,
            default_user_role: None,
        }
    }

    /// Built-in roles: `owner` = everything; `member` = everything except
    /// `admin`; `guest` = nothing (share grants supply guest rules).
    pub fn default_roles() -> std::collections::HashMap<String, Vec<Rule>> {
        let mut m = std::collections::HashMap::new();
        m.insert("owner".into(), vec![Rule::new("**", &["*"])]);
        m.insert(
            "member".into(),
            vec![Rule::new(
                "**",
                &[Action::READ, Action::WRITE, Action::COMMENT, Action::DOWNLOAD],
            )],
        );
        m.insert("guest".into(), Vec::new());
        m
    }

    /// Replace/add a role's rules (e.g. parsed from `permissions_json`).
    pub fn set_role(&mut self, role: impl Into<String>, rules: Vec<Rule>) {
        self.roles.insert(role.into(), rules);
    }

    /// Parse a `permissions_json` blob (`[{"resource": "...", "actions": [...]}]`).
    pub fn role_from_json(
        &mut self,
        role: impl Into<String>,
        json: &str,
    ) -> Result<(), serde_json::Error> {
        let rules: Vec<Rule> = serde_json::from_str(json)?;
        self.set_role(role, rules);
        Ok(())
    }

    /// Assume this role for validated users missing a membership row.
    pub fn with_default_user_role(mut self, role: impl Into<String>) -> Self {
        self.default_user_role = Some(role.into());
        self
    }

    /// Set the member table (`user_id → role`).
    pub fn set_member(&mut self, user_id: impl Into<String>, role: impl Into<String>) {
        self.members.insert(user_id.into(), role.into());
    }

    fn rules_for(&self, who: &Principal) -> Option<&[Rule]> {
        match who {
            Principal::User { user_id } => self
                .members
                .get(user_id)
                .or(self.default_user_role.as_ref())
                .and_then(|role| self.roles.get(role))
                .map(|v| v.as_slice()),
            _ => None,
        }
    }
}

impl PermissionEngine for RoleEngine {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        if self.allow_services {
            if let Principal::Service { .. } = who {
                return Decision::Allow;
            }
        }
        match self.rules_for(who) {
            Some(rules) if rules.iter().any(|r| r.permits(what, action)) => Decision::Allow,
            Some(_) => Decision::deny(format!(
                "{} may not {} {}",
                who.describe(),
                action.as_str(),
                what.as_str()
            )),
            None => Decision::deny(format!("{} is not a member", who.describe())),
        }
    }

    fn survey(&self, who: &Principal, prefix: &Resource) -> Vec<(Resource, Vec<Action>)> {
        let Some(rules) = self.rules_for(who) else {
            return Vec::new();
        };
        rules
            .iter()
            .filter(|r| prefix.as_str().is_empty() || r.resource.starts_with(prefix.as_str()))
            .map(|r| {
                (
                    Resource::new(r.resource.clone()),
                    r.actions.iter().map(|a| Action::new(a.clone())).collect(),
                )
            })
            .collect()
    }
}

/// First-Allow wins across children; all-Deny returns the FIRST deny (the
/// most authoritative reason). Empty composite denies everything.
#[derive(Clone, Default)]
pub struct CompositeEngine {
    engines: Vec<Arc<dyn PermissionEngine>>,
}

impl CompositeEngine {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(mut self, engine: Arc<dyn PermissionEngine>) -> Self {
        self.engines.push(engine);
        self
    }
}

impl PermissionEngine for CompositeEngine {
    fn check(&self, who: &Principal, what: &Resource, action: &Action) -> Decision {
        let mut first_deny = None;
        for e in &self.engines {
            match e.check(who, what, action) {
                Decision::Allow => return Decision::Allow,
                deny => {
                    if first_deny.is_none() {
                        first_deny = Some(deny);
                    }
                }
            }
        }
        first_deny.unwrap_or_else(|| Decision::deny("no permission engine allowed the request"))
    }

    fn survey(&self, who: &Principal, prefix: &Resource) -> Vec<(Resource, Vec<Action>)> {
        self.engines
            .iter()
            .flat_map(|e| e.survey(who, prefix))
            .collect()
    }
}

/// Allow-everything engine — the migration default for lanes that are not
/// yet enforcing (observe/audit-only mode).
#[derive(Clone, Copy, Debug, Default)]
pub struct AllowAll;

impl PermissionEngine for AllowAll {
    fn check(&self, _who: &Principal, _what: &Resource, _action: &Action) -> Decision {
        Decision::Allow
    }
}

// ─── Method permits (the router gate's rule table) ───────────────────────

/// What one RPC method requires. `resource` may reference decoded argument
/// fields as `{field}` (interpolated reflectively by the gate when the
/// transport exposes decoded args) — a method-level gate that can't
/// interpolate falls back to the literal template with `{field}` intact,
/// so patterns should put wildcards AFTER any interpolated tail
/// (`vault/{path}` is checked as `vault/**` at method level).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodPermit {
    pub method: &'static str,
    pub action: &'static str,
    /// Resource template; `{arg}` interpolates the like-named argument.
    pub resource: &'static str,
    /// Emit an audit event on Allow too (downloads, destructive ops).
    pub audit: bool,
}

impl MethodPermit {
    pub const fn new(method: &'static str, action: &'static str, resource: &'static str) -> Self {
        Self {
            method,
            action,
            resource,
            audit: false,
        }
    }
    pub const fn audited(mut self) -> Self {
        self.audit = true;
        self
    }

    /// The method-level (pre-decode) resource: the template with every
    /// `{arg}` interpolation widened to `**`.
    pub fn coarse_resource(&self) -> Resource {
        if !self.resource.contains('{') {
            return Resource::new(self.resource);
        }
        let head = self
            .resource
            .split('{')
            .next()
            .unwrap_or("")
            .trim_end_matches('/');
        if head.is_empty() {
            Resource::new("**")
        } else {
            Resource::new(format!("{head}/**"))
        }
    }
}

/// A service's permit table — what each method needs. Methods NOT listed
/// are DENIED when the table is installed (fail-closed).
#[derive(Clone, Debug)]
pub struct ServicePermits {
    pub service: &'static str,
    pub methods: &'static [MethodPermit],
}

// ─── Audit ───────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct AuditEvent {
    pub principal: String,
    pub resource: String,
    pub action: String,
    pub allowed: bool,
    pub reason: Option<String>,
}

pub trait AuditSink: MaybeSendSync {
    fn record(&self, event: AuditEvent);
}

/// Audit into `tracing` (info allows / warn denies) — the default sink.
#[derive(Clone, Copy, Debug, Default)]
pub struct TracingAudit;

impl AuditSink for TracingAudit {
    fn record(&self, e: AuditEvent) {
        if e.allowed {
            tracing::info!(
                principal = %e.principal, resource = %e.resource, action = %e.action,
                "permission allow"
            );
        } else {
            tracing::warn!(
                principal = %e.principal, resource = %e.resource, action = %e.action,
                reason = e.reason.as_deref().unwrap_or(""),
                "permission DENY"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: &str) -> Principal {
        Principal::User {
            user_id: id.into(),
        }
    }

    #[test]
    fn glob_basics() {
        assert!(glob_matches("**", "anything/at/all"));
        assert!(glob_matches("vault/**", "vault/Songs/Praise.md"));
        assert!(glob_matches("vault/**", "vault"));
        assert!(!glob_matches("vault/**", "media/abc"));
        assert!(glob_matches("vault/*/chart.kf", "vault/praise/chart.kf"));
        assert!(!glob_matches("vault/*/chart.kf", "vault/a/b/chart.kf"));
        assert!(glob_matches("vault/Songs/", "vault/Songs/Praise.md"));
        assert!(glob_matches("vault/Songs/", "vault/Songs"));
        assert!(!glob_matches("vault/Songs/", "vault/Songs2/x.md"));
        assert!(!glob_matches("", "vault"));
        // `**` mid-pattern
        assert!(glob_matches("vault/**", "vault/a/b/c"));
        assert!(glob_matches("service/*/read", "service/media/read"));
    }

    #[test]
    fn scope_engine_allows_inside_denies_outside() {
        let e = ScopeEngine::new(vec![
            Rule::new("vault/Setlists/Sunday Worship.md", &["read", "comment"]),
            Rule::new("resources/songs/", &["read"]),
        ]);
        let p = Principal::Guest {
            link_id: "l1".into(),
            display: None,
        };
        assert!(e
            .check(&p, &"vault/Setlists/Sunday Worship.md".into(), &Action::read())
            .allowed());
        assert!(e
            .check(&p, &"resources/songs/praise/stems/01.ogg".into(), &Action::read())
            .allowed());
        assert!(!e
            .check(&p, &"vault/Finance/Q3.md".into(), &Action::read())
            .allowed());
        assert!(!e
            .check(&p, &"resources/songs/praise/stems/01.ogg".into(), &Action::write())
            .allowed());
    }

    #[test]
    fn role_engine_owner_member_guest() {
        let mut e = RoleEngine::new();
        e.set_member("alice", "owner");
        e.set_member("bob", "member");
        e.set_member("visitor", "guest");

        assert!(e.check(&user("alice"), &"vault/x".into(), &Action::admin()).allowed());
        assert!(e.check(&user("bob"), &"vault/x".into(), &Action::write()).allowed());
        assert!(!e.check(&user("bob"), &"vault/x".into(), &Action::admin()).allowed());
        assert!(!e.check(&user("visitor"), &"vault/x".into(), &Action::read()).allowed());
        assert!(!e.check(&user("nobody"), &"vault/x".into(), &Action::read()).allowed());
        assert!(!e
            .check(&Principal::Anonymous, &"vault/x".into(), &Action::read())
            .allowed());
        assert!(e
            .check(
                &Principal::Service { name: "engine".into() },
                &"vault/x".into(),
                &Action::write()
            )
            .allowed());
    }

    #[test]
    fn default_user_role_covers_unlisted_users() {
        let mut e = RoleEngine::new().with_default_user_role("member");
        e.set_member("visitor", "guest");
        // Unknown-but-validated user falls back to member…
        assert!(e.check(&user("someone"), &"vault/x".into(), &Action::write()).allowed());
        // …explicit rows still win…
        assert!(!e.check(&user("visitor"), &"vault/x".into(), &Action::read()).allowed());
        // …and anonymous stays denied.
        assert!(!e
            .check(&Principal::Anonymous, &"vault/x".into(), &Action::read())
            .allowed());
    }

    #[test]
    fn role_from_json_blob() {
        let mut e = RoleEngine::new();
        e.role_from_json(
            "librarian",
            r#"[{"resource": "vault/Songs/", "actions": ["read", "write"]},
                {"resource": "resources/scores/", "actions": ["read", "download"]}]"#,
        )
        .unwrap();
        e.set_member("kim", "librarian");
        assert!(e
            .check(&user("kim"), &"vault/Songs/Time.md".into(), &Action::write())
            .allowed());
        assert!(e
            .check(
                &user("kim"),
                &"resources/scores/time/score.musicxml".into(),
                &Action::new("download")
            )
            .allowed());
        assert!(!e.check(&user("kim"), &"vault/Finance/x".into(), &Action::read()).allowed());
    }

    #[test]
    fn composite_first_allow_wins_and_first_deny_reported() {
        let scope = Arc::new(ScopeEngine::new(vec![Rule::new("vault/a", &["read"])]));
        let mut roles = RoleEngine::new();
        roles.set_member("alice", "member");
        let composite = CompositeEngine::new()
            .push(Arc::new(roles))
            .push(scope);
        // member → allowed by roles even though scope would deny
        assert!(composite
            .check(&user("alice"), &"vault/zzz".into(), &Action::read())
            .allowed());
        // guest → roles deny, scope allows vault/a
        let g = Principal::Guest { link_id: "l".into(), display: None };
        assert!(composite.check(&g, &"vault/a".into(), &Action::read()).allowed());
        let d = composite.check(&g, &"vault/b".into(), &Action::read());
        assert!(!d.allowed());
        // empty composite denies
        assert!(!CompositeEngine::new()
            .check(&Principal::Anonymous, &"x".into(), &Action::read())
            .allowed());
    }

    #[test]
    fn method_permit_coarse_resource() {
        assert_eq!(
            MethodPermit::new("get_file", "read", "vault/{path}").coarse_resource(),
            Resource::new("vault/**")
        );
        assert_eq!(
            MethodPermit::new("build", "admin", "service/setlist/build").coarse_resource(),
            Resource::new("service/setlist/build")
        );
        assert_eq!(
            MethodPermit::new("read", "read", "{hash}").coarse_resource(),
            Resource::new("**")
        );
    }
}
