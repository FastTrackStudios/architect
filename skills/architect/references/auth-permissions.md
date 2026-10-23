# Auth and permissions: the architect API

Scope: `features/permissions` (+ `features/permissions/proto`), `crates/architect/src/permissions_gate.rs`, `features/auth/*`, `apps/auth-server`, `apps/mock-oauth`. The deployed issuer (Argo, secrets) is `references/fts-auth.md`. Guide: `docs/content/architecture/auth.md`.

In short: (1) the gate is fail-closed by default, and observe-only is the migration mode that lets everything through; (2) a consumer writes one `ServicePermits` const per service plus one `PermissionsGate` builder; (3) the caller is a `Principal` from the bearer via `SessionIdentityResolver`, `Anonymous` by default; (4) `PermissionsService::can` ignores the caller.

## Pieces

| Piece | What it does | Entry points |
|---|---|---|
| `architect-permissions` | Principal × Resource × Action → Decision; globs; Role/Scope/Composite engines; audit | `features/permissions/src/lib.rs` `PermissionEngine`, `IdentityResolver`, `RoleEngine`, `ScopeEngine`, `CompositeEngine`, `MethodPermit`, `ServicePermits`, `AuditSink` |
| `architect-permissions-proto` | `PermissionsService` affordance oracle: `can`, `capabilities` | `features/permissions/proto/src/lib.rs` `PermissionsService`, `Permissions` |
| Permissions gate (vox, native) | method-level router gate, deny reply, coverage, `caller()` | `crates/architect/src/permissions_gate.rs` `PermissionsGate`, `UnlistedPolicy`, `GateCoverage`, `caller` |
| `architect-auth` | umbrella: re-exports `auth` + `auth_proto`; features db/axum/vox/client/full | `features/auth/architect-auth/src/lib.rs` `stored_session` |
| `auth` | engine: `ArchitectAuth<S>`, flows, plugins, storage, axum + vox transports, identity | `features/auth/auth/src/lib.rs` `ArchitectAuth`; `features/auth/auth/src/identity.rs` `SessionIdentityResolver`; `features/auth/auth/src/transport.rs` |
| `auth-proto` | wire types; `AuthService`, `OrganizationService`, `DeviceService` | `features/auth/auth-proto/src/service.rs` `AuthService`; `features/auth/auth-proto/src/device.rs` `DeviceService` |
| `auth-db` | SeaORM backend: migrations, snapshot | `features/auth/auth-db/src/lib.rs` |
| `auth-client` | wasm-clean client kit: token stores, middleware, OIDC PKCE, RFC 8628 device flow | `features/auth/auth-client/src/lib.rs` `TokenStore`, `FileTokenStore`, `TokenStoreMiddleware`; `features/auth/auth-client/src/oidc.rs`; `features/auth/auth-client/src/device.rs` |
| `auth-ui` | server-rendered account/org/passkey/2FA pages, no script | `features/auth/auth-ui/src/lib.rs` `router` |
| Integration tests | gate over a real vox memory link; device address book | `features/auth/tests/native/src/permissions_gate_tests.rs`; `features/auth/tests/native/src/devices_tests.rs` |
| `apps/auth-server` | deployable server: `/vox`, iroh, HTTP `/auth/*`, OIDC, hosted pages | `apps/auth-server/src/lib.rs` `build`; `apps/auth-server/src/main.rs` |
| `apps/mock-oauth` | stand-in GitHub/Google/TONE3000 for social sign-in in dev and tests | `apps/mock-oauth/src/lib.rs` |

## How the gate decides a request

1. Method found in a table: resolve the principal once, then `engine.check(who, coarse_resource, action)`.
2. Allow: audit only if the permit is `.audited()`; the handler runs inside the `CALLER` task-local scope.
3. Deny: reply `permission denied: <reason> (<service>/<method>)`, built in the method's own response shape so the client decodes the reason.
4. A method of a tabled service that the table omits: fail-closed deny (audited `method not in permit table (fail-closed)`), even under `UnlistedPolicy::Allow`.
5. A service with no table: `UnlistedPolicy::Allow` passes; `Deny` (the default) audits `service not mounted for this lane` and denies.

## Resources and actions

- Resources are `/`-separated: `vault/<path>`, `media/<hash>`, `doc/<id>`, `threads/<type>/<id>`, `service/<name>/<method>`. Action Lists check `action/<id>` with `run` (`action_resource`, `Action::RUN`).
- Actions: `read`, `write`, `comment`, `admin`, `download`, `run`; free-form strings allowed; a rule `"*"` matches any action.
- Globs: `*` = one segment, `**` = any tail (last only), trailing `/` = prefix form, empty pattern matches nothing.
- `MethodPermit::new(method, action, resource)` takes `&'static str`s, so tables are `const`. `{arg}` in a resource widens to `**` at the gate.

## Coverage and audit

- `GateCoverage`: `services`, `tabled`, `untabled`, `methods`, `permitted`, `uncovered` (on the descriptor, missing from its table: denied), `phantom` (named by a table, absent from the descriptor). `is_complete()` = no untabled, uncovered or phantom. `report()` logs one line per gap class (`SERVED UNCHECKED` for untabled under `Allow`).
- `wrap` prints the report itself; `wrap_shared` and `wrap_handler` do not, so call `report()` yourself.
- `AuditSink::record(AuditEvent { .. })`; default `TracingAudit` (info on allow, warn on deny); swap with `PermissionsGate::with_audit`.

## A. Add sign-in to an app

1. Reach an auth server. Public OIDC client: `oidc::authorize_url`, `Pkce::from_entropy`, `check_state`, `token_request_body`, userinfo `user_from`. Headless: RFC 8628 `device::start_body`, `parse_start`, `poll_body`, `parse_poll`. The client is protocol-only: the app posts with its own HTTP client.
2. Persist: `architect_auth::client::stored_session(&bundle)`, or for an OIDC sign-in `oidc::token_response_from(body)` then `TokenResponse::into_session(now_unix)` (keeps the `offline_access` refresh token in `StoredSession::refresh_token`); then `FileTokenStore::new(path)` (0600 via temp + rename, refresh token in the same file) or `MemoryTokenStore` (wasm, tests).
   Stay signed in past the access token's hour: `oidc::refresh_if_needed(&store, client_id, now_unix, send)` before a call (renews within `REFRESH_MARGIN_SECS`), `oidc::refresh_now` after a 401. `send` posts `oidc::refresh_request_body` to `oidc::TOKEN_PATH` with the app's HTTP and returns the body. Both save the rotated session; `RefreshError::Oidc(OidcError::Refused { .. })` (e.g. `invalid_grant`) means sign in again. Refresh tokens rotate, so run one refresh per store at a time. Tests: `features/auth/auth-client/src/oidc.rs` `a_due_token_is_rotated_and_saved`.
3. Attach: `TokenStoreMiddleware::new(store)` loads the store on every call and pushes `authorization: Bearer <token>`; or `AuthClientMiddleware::bearer(token)`. Example: `features/auth/architect-auth/examples/vox_rpc.rs`.
4. A plain HTTP app's server side: `require_session` as a `route_layer`, then `Extension<AuthenticatedSession>` (`user_id`, `session_id`, `token`). Example: `features/auth/architect-auth/examples/axum_http.rs`. Both examples need feature `full`.

## B. Mount the gate and write permit tables

1. Engine and identity: e.g. `RoleEngine::new().with_default_user_role("member")` inside a `CompositeEngine` with a `ScopeEngine` granting `public/**` (Task's org lane). Identity: `SessionIdentityResolver::new(auth.clone())` (30 s cache; `with_ttl(Duration::ZERO)` disables it).
2. Table: `const X: ServicePermits = ServicePermits { service: "...", methods: &[MethodPermit::new("method", Action::READ, "vault/{path}")] };` plus `.audited()` where wanted.
3. Gate: `PermissionsGate::new(engine, Arc::new(identity)).unlisted(UnlistedPolicy::Deny).observe_only(!enforce).permit(<svc>_service_descriptor(), &TABLE)`, one `permit` per service; the descriptor comes from the service macro.
4. Install: `router.with_permissions(gate)`, or per connection `PermissionsGate::wrap_shared_with_bearer(Arc::new(gate), router, bearer)`.
5. Check: `coverage(&router).is_complete()` in a test.
6. Serve the oracle from the same pair: `Permissions::new(gate.engine(), gate.identity_resolver())` + `PermissionsServiceDispatcher`, so it cannot disagree with enforcement.
7. Handlers see the caller with `architect::permissions_gate::caller()` (`Option<Principal>`); test `a_handler_sees_the_caller_the_gate_resolved`. Action List handlers get it as `ActionCaller` instead.

## C. How a client authenticates, and the Principal that results

- The client sends `authorization: Bearer <token>` metadata (`AUTHORIZATION_METADATA_KEY`). Browsers can't set WebSocket headers: the token rides the handshake as the connection bearer.
- Per-call metadata wins over the connection bearer, so one connection can make a call as a different principal. Parsing is `strip_prefix("Bearer ")` then trim.
- `SessionIdentityResolver::resolve`: no/empty token → `Anonymous`; valid session → `Principal::User { user_id }`; any error → `Anonymous`.

| Principal | Produced by | Rights in `RoleEngine` |
|---|---|---|
| `User { user_id }` | validated session | member role if in `members`, or the default role |
| `Anonymous` | no or invalid token | none; denied unless a scope/public rule matches |
| `Guest { link_id, display }` | share lane `StaticPrincipal` | none (the share engine supplies rules) |
| `Service { name }` | in-process caller | `RoleEngine` allows all |
| `Host { endpoint }` | transport-proven peer endpoint | none on its own |

Tests to copy: `features/auth/tests/native/src/permissions_gate_tests.rs` (`establish_gated`, observe-only, untabled denied, `coverage_names_every_gap`); `apps/auth-server/tests/surfaces.rs` (one suite across HTTP, pooled WebSocket and iroh; org isolation, invitations).

## D. Other principals

- Share lane: `StaticPrincipal(Principal::Guest { link_id, display })` with `ScopeEngine::new(vec![Rule::new("probe/", &["read"])])`; a scope denies everything outside its rules.
- Identity chains (Task): host bearer → central accounts → home-org fallback → own store, wrapped in an audited resolver so "no token" and "token rejected" are distinguishable (`task:apps/server/src/permits.rs`, `task:apps/server/src/lib.rs`). Central identity alone never admits: the membership table is the fence.

## Pitfalls

- **Fail-closed by default; Task opts out.** Task sets `UnlistedPolicy::Allow`, so its untabled services are served unchecked, and enforces only when `TASK_ENFORCE_PERMISSIONS` is exactly `1` (`task:apps/server/src/config.rs`).
- **Observe-only lets every deny through**, including fail-closed and unlisted denials; it audits `principal: "observe-only", action: "would-deny"`, and resolves identity twice on a deny.
- **The gate checks the coarse resource only** (`vault/{path}` means `vault/**`); per-path decisions belong in the handler via `PermissionEngine::check`.
- **`PermissionsService::can` always checks `Anonymous`.** Use `capabilities(token, prefix)` for a person. Both are advisory; an empty survey means unknown, not no access.
- **Roles:** `member` = read/write/comment/download; `owner` = `*`; `guest` = nothing; `admin` is in no default grant.
- **Service bypass:** `RoleEngine` allows every `Principal::Service` (`allow_services: true`). Never give untrusted callers that variant. `Host` gets no bypass.
- **Anonymous is denied everywhere by `RoleEngine`:** put sign-in on a public scope (Task: `public/**` on `AuthService`).
- **Cookies don't reach the gate;** only metadata and the connection bearer do. The axum path also accepts a session cookie.
- **Revocation lags up to 30 s** (the resolver caches `Anonymous` too). Sign-out is not instant.
- **A malformed token comes back as `Anonymous`** with no distinct error.
- **Phantom permits:** a table naming a method the descriptor lacks is `phantom`; the real method is denied.
- **`caller()` is task-local:** work spawned onto another task sees `None` and must refuse.
- **Gate state is shared** (`wrap_shared` takes an `Arc`); per-connection state is only the connection bearer. Spans exist only with feature `telemetry`.
- **wasm:** the gate and `caller()` are excluded on wasm32; the engine crate switches its `Send` bounds; `auth-client` is wasm-clean.
- **Two authorization planes:** the router gate does not cover in-process `ActionList` runs. Give the list the gate's engine with `set_authorizer`.
- **mock-oauth is not an OAuth server** (no client secret, PKCE or expiry checks). Never point a production auth server at it.
- **Auth-server JWTs are HS256 with an empty JWKS:** only the issuer holds the secret, so no app, client or peer can validate them offline (consequences in `references/fts-auth.md`). Remote token introspection (I34) is not in architect yet; Task's `task:apps/server/src/central_auth.rs` `CentralAuth` is the only implementation.
