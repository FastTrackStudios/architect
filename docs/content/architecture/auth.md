+++
title = "The auth feature"
description = "architect-auth — the session RPC surface, the server middleware, the client session kit, and the engine underneath."
weight = 53
+++

Auth lives in `features/auth/` as its own crate family — there is **no
`architect::auth` re-export** (it would cycle the dependency graph;
depend on the auth crates directly). The split follows the
[monorepo layout](@/architecture/layout.md):

| Crate | Role |
| --- | --- |
| `auth-proto` | wire types (`AuthUser`, `AuthSession`, `AuthSessionBundle`, payloads, `AuthFlowError`) + the `#[architect::rpc] AuthService` trait |
| `auth` | the engine: `ArchitectAuth<S>` over a pluggable `AuthStorage`, ~30 plugins' worth of flows, the vox + axum transports |
| `auth-db` | SeaORM storage + migrations |
| `auth-client` | the client session kit — tiny, **wasm-clean**, no engine dependency |
| `architect-auth` | the umbrella: `pub use auth::*`, plus `db` / `client` modules behind features (`db`, `axum`, `vox`, `client`, `full`) |

## The RPC surface

`AuthService` covers exactly the session lifecycle — all-async, so it
mounts like any other architect service:

```rust,ignore
#[architect::rpc]
pub trait AuthService {
    async fn sign_up_email_password(&self, input: SignUpEmailPassword) -> Result<AuthSessionBundle, AuthFlowError>;
    async fn sign_in_email_password(&self, input: SignInEmailPassword) -> Result<AuthSessionBundle, AuthFlowError>;
    async fn current_session(&self, token: String) -> Result<AuthSessionBundle, AuthFlowError>;
    async fn refresh_session(&self, token: String) -> Result<AuthSessionBundle, AuthFlowError>;
    async fn whoami(&self, token: String) -> Result<AuthUser, AuthFlowError>;
    async fn sign_out(&self, token: String) -> Result<(), AuthFlowError>;
}
```

The `AuthSessionBundle` a sign-up/sign-in/refresh returns carries the
raw token (only its hash is stored server-side), the `AuthUser`, and
the `AuthSession`. Refresh **rotates**: new token, new expiry, old
token dead. Sign-out is idempotent and doesn't reveal token existence.

## Server: build the engine, mount the service

`AuthVoxService` adapts the engine to the trait; the rpc-emitted
`auth_service_layer` binds it into a `LayerRouter`. This is verbatim
how the integration tests mount it
(`features/auth/tests/native`, over the
[in-process transport](@/architecture/local.md) — full wire, no
socket):

```rust,ignore
use architect::{LayerRouter, LocalServer, Scope};
use auth::transport::vox::AuthVoxService;
use auth::{AuthServiceClient, SignUpEmailPassword};
use auth::backend_db::{AuthSeaOrmStorage, Migrator};

let db = Database::connect("sqlite::memory:").await?;
Migrator::up(&db, None).await?;
let auth = auth::ArchitectAuth::builder()
    .secret("a-secret-at-least-32-bytes-long!!")   // < 32 bytes is rejected
    .storage(AuthSeaOrmStorage::new(db))
    .build()?;

let router = LayerRouter::new()
    .merge(auth::auth_service_layer(AuthVoxService::new(auth)));

// in production: serve over axum_ws; here, in-process
let scope = Scope::new();
let local = LocalServer::serve(router, scope.clone());
let client: AuthServiceClient = local.establish().await?;

let bundle = client.sign_up_email_password(SignUpEmailPassword {
    email: "rpc@example.com".into(),
    password: "correct horse battery staple".into(),
    ..Default::default()    // schematic — spell the remaining fields out
}).await?;
let me = client.whoami(bundle.token.clone()).await?;
assert_eq!(me.id, bundle.user.id);
```

### Protecting *other* services: `AuthServerMiddleware`

Auth crosses the wire as vox string metadata —
`authorization: Bearer <token>` under `AUTHORIZATION_METADATA_KEY`.
`AuthServerMiddleware` parses it on every request and stashes an
`AuthVoxContext { token: Option<String> }` in the request extensions;
your service reads it via `#[vox::context]` and validates with
`auth.current_session(...)`:

```rust,ignore
use auth::transport::vox::{AuthServerMiddleware, AuthVoxContext};

let router = LayerRouter::new().with(
    tasks_service_descriptor(),
    TasksDispatcher::new(tasks).with_middleware(AuthServerMiddleware),
);

// inside a #[vox::context] method:
async fn list(&self, cx: &vox::RequestContext<'_>) -> Result<Vec<Task>, TaskError> {
    let token = cx.extensions()
        .get_cloned::<AuthVoxContext>()
        .and_then(|c| c.token)
        .ok_or(TaskError::Unauthenticated)?;
    let session = self.auth.current_session(CurrentSession { token }).await?;
    // … session.user.id is your caller …
}
```

A missing header is an *unauthenticated* call, not a failed one — the
context's token is simply `None`, and the policy is the service's.

## Client: the session kit (`auth-client`)

Everything a downstream app needs to hold a session and present it on
later calls — wasm-clean, engine-free:

- **`StoredSession`** — token + optional user id / email / expiry.
  `architect_auth::client::stored_session(&bundle)` builds one from a
  sign-in's `AuthSessionBundle`.
- **`TokenStore`** — save/load/clear behind a trait.
  **`FileTokenStore`** (native): one JSON file, atomic write
  (temp + rename), `0600` on unix — the token is a bearer credential.
  **`MemoryTokenStore`**: wasm and tests.
- **`TokenStoreMiddleware`** — a `vox::ClientMiddleware` that loads the
  store **per call** (a re-login is picked up without rebuilding
  clients) and attaches `Bearer <token>` exactly as
  `AuthServerMiddleware` parses it, flagged `SENSITIVE | NO_PROPAGATE`.

```rust,ignore
use auth_client::{FileTokenStore, TokenStoreMiddleware};
use architect_auth::client::stored_session;

let store = Arc::new(FileTokenStore::new(dirs.data_dir().join("session.json")));

// after login:
store.save(&stored_session(&bundle))?;

// every typed vox client picks the session up automatically:
let tasks = tasks_client.with_middleware(TokenStoreMiddleware::new(store.clone()));

// logout:
store.clear()?;
```

The wire format is pinned by an integration test that round-trips
`TokenStoreMiddleware` against the real `AuthServerMiddleware` over a
`LocalServer` — empty store → server sees no token; saved token →
server sees it verbatim; cleared → none again, same clients.

## Beyond the RPC surface

The six-method trait is deliberately minimal. The engine underneath
speaks a much larger command vocabulary — **engine-level only**, called
on `ArchitectAuth` directly (server-side), not mounted on the wire:

- **Two-factor**: `StartTwoFactorSetup` / `ConfirmTwoFactor` /
  `VerifyTwoFactor` / `DisableTwoFactor` (TOTP + backup codes).
- **API keys**: `CreateApiKey` / `VerifyApiKey` / `AuthenticateApiKey`
  / `RevokeApiKey` — machine credentials with their own authorize path.
- **Invitations & organizations**: `CreateInvitation` /
  `AcceptInvitation`, organizations, teams, roles, member management.
- Plus passkeys, OAuth/OIDC, magic links, email/phone OTP, device
  authorization, anonymous sign-in, impersonation, JWT issuance, … —
  see the `AUTH_PLUGIN_DESCRIPTORS` table in `auth::plugins`.

Exposing any of these to clients later is purely additive — new trait
methods (or a second `#[architect::rpc]` trait), no re-mount of what
exists.

There's also an **axum HTTP guard** for plain HTTP routes
(`auth::transport::axum`): `require_session` as a `route_layer`
extracts a bearer token or session cookie and injects an
`AuthenticatedSession` extension — see
`features/auth/architect-auth/examples/axum_http.rs`. The vox path
above is the architect-idiomatic one; reach for the axum guard only on
routes that genuinely live outside vox.
