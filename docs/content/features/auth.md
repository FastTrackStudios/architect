+++
title = "Auth"
description = "Accounts and sessions: the auth engine, its services on every transport, the client kit (token store, OIDC with PKCE, device flow), and apps/auth-server."
weight = 30
+++

## What it is and when to use it

Auth is a family of crates under `features/auth`. `auth-proto` declares
`AuthService`, `OrganizationService` and `DeviceService` once, as
`#[architect::service]` traits, so each has a vox face, an HTTP face
(`POST /auth/<method>`) and clients for both. `auth` is the engine
(`ArchitectAuth`): accounts, sessions (only a token's hash is stored),
organizations, devices, OIDC provider, device flow, passkeys and two-factor.
`auth-client` is the wasm-clean client kit: a stored session, token stores,
middleware that attaches the bearer to every vox call, OIDC with PKCE and
refresh, and the RFC 8628 device flow. `apps/auth-server` is the deployable
server (deployed as fts-auth).

Use it to sign people in and to know who is calling. What a caller may do is
[Permissions](@/features/permissions.md). There is no `architect::auth`
re-export; depend on the crates. Design: [Auth](@/architecture/auth.md);
the deployment is in `skills/architect/references/fts-auth.md`.

## A minimal example

`architect-auth` with feature `full`. From
`features/auth/architect-auth/examples/vox_rpc.rs`:

```rust
use architect::{LayerRouter, LocalServer, Scope};
use architect_auth::{ArchitectAuth, AuthServiceClient, SignUpEmailPassword, auth_service_layer,
    client::{stored_session, FileTokenStore, TokenStore, TokenStoreMiddleware},
    db::{AuthSeaOrmStorage, Migrator}, transport::vox::AuthVoxService};
use sea_orm_migration::MigratorTrait;

let db = sea_orm::Database::connect("sqlite::memory:").await?;
Migrator::up(&db, None).await?;
let auth = ArchitectAuth::builder()
    .secret("a-secret-at-least-32-bytes-long!!")      // shorter is refused
    .storage(AuthSeaOrmStorage::new(db))
    .build()?;

let router = LayerRouter::new().merge(auth_service_layer(AuthVoxService::new(auth)));
let scope = Scope::new();
let client: AuthServiceClient = LocalServer::serve(router, scope.clone()).establish().await?;

let bundle = client.sign_up_email_password(SignUpEmailPassword {
    email: "a@b.c".into(), password: "correct horse battery staple".into(),
    name: None, username: None, image: None, metadata_json: None,
    ip_address: None, user_agent: None,
}).await?;

let store = std::sync::Arc::new(FileTokenStore::new("session.json"));
store.save(&stored_session(&bundle))?;
// Every later client: client.with_middleware(TokenStoreMiddleware::new(store.clone()))
```

A server protects its own services with `AuthServerMiddleware` (the handler
reads `AuthVoxContext`), or resolves the bearer for the permissions gate with
`SessionIdentityResolver`. Plain axum routes use `require_session`
(`features/auth/architect-auth/examples/axum_http.rs`).

## Code tour

1. `features/auth/auth-proto/src/service.rs` `AuthService`: sign up and in,
   device sign-in, `current_session`, `refresh_session`, `whoami`, `sign_out`,
   profile changes.
2. `features/auth/auth-proto/src/organizations.rs` `OrganizationService` and
   `features/auth/auth-proto/src/device.rs` `DeviceService`: orgs, members,
   invitations, roles; a person's devices.
3. `features/auth/auth/src/config.rs` `ArchitectAuthBuilder` and
   `features/auth/auth/src/lib.rs` `ArchitectAuth`: the engine.
4. `features/auth/auth/src/transport.rs` `AuthVoxService`,
   `AuthServerMiddleware`, `AuthVoxContext`: the engine as the traits, and the
   server side of the bearer.
5. `features/auth/auth/src/identity.rs` `SessionIdentityResolver`: a bearer to
   a `Principal`, cached briefly.
6. `features/auth/auth-client/src/lib.rs` `StoredSession`, `TokenStore`,
   `FileTokenStore`, `MemoryTokenStore`, `TokenStoreMiddleware`.
7. `features/auth/auth-client/src/oidc.rs` `authorize_url`, `Pkce`,
   `refresh_if_needed`; `features/auth/auth-client/src/device.rs` `start_body`,
   `poll_body`.
8. `apps/auth-server/src/server.rs` `services`, `build`, `serve`: the three
   services on every face; `apps/auth-server/src/oauth.rs` the OIDC routes and
   hosted pages.

A client signs in through `AuthServiceClient` or `AuthServiceHttpClient`.
`AuthVoxService` calls the engine, which returns a session bundle. The app
keeps it with `stored_session` in a `TokenStore`; `TokenStoreMiddleware`
reads the store on every call and sends `authorization: Bearer <token>`, so a
refresh lands on the next call. On the server, `AuthServerMiddleware` turns
that into an `AuthVoxContext`, or `SessionIdentityResolver` into a
`Principal` for the gate. A device without a browser posts the device-code
request, shows the code, and polls until the person approves it elsewhere.

### A native app's sign-in

The Architect app signs in in the system's browser, on the issuer's own
page, never in fields it draws (`features/accounts/src/redirect.rs`,
`features/accounts/src/browser.rs`): authorization code with PKCE (S256)
and a checked `state`. On iOS the page opens in `ASWebAuthenticationSession`
and comes back on the app's scheme, `app.fasttrackstudio.architect:/oauth/callback`;
on a desk it opens in the default browser and comes back on a loopback
port the app listens on, `http://127.0.0.1:<port>/oauth/callback`. The
issuer matches a registered loopback redirect on any port (RFC 8252 §7.3,
`features/auth/auth/src/flows.rs` `redirect_matches`), never `localhost`.

The code buys an OIDC access token, which the issuer's vox services do not
take: they take sessions. So the app trades it at once for an issuer
session (RFC 8693 at `/oauth2/token`, grant type `TOKEN_EXCHANGE`,
`requested_token_type` `SESSION_TOKEN_TYPE`, `ExchangeForSession`), allowed
only to a client registered with `issue_sessions`, for a token issued to
that client in the last `EXCHANGE_WINDOW_SECONDS`, once per token; the
session is new (signing the app out leaves the browser signed in). From
there it is the session it always was: kept in the keychain, renewed,
revoked on sign-out.

The `architect` client the native app uses, in `AUTH_OIDC_CLIENTS`:

```json
{ "client_id": "architect", "public_client": true, "skip_consent": true,
  "issue_sessions": true,
  "scopes": ["openid", "profile", "email", "offline_access"],
  "redirect_uris": ["http://127.0.0.1/oauth/callback",
                    "app.fasttrackstudio.architect:/oauth/callback",
                    "https://architect.fasttrackstudio.app/auth/callback"] }
```

The device-code flow stays for what has no browser to come back to (a
headless node, the CLI: `Accounts::sign_in_with_code`).

## Tests to read

- `apps/auth-server/tests/surfaces.rs`: one scenario over HTTP, in-process
  vox, WebSocket and iroh: sessions, a wrong password, org isolation,
  invitations and roles, orgs fetched live.
- `features/auth/tests/native/src/lib.rs`: the session surface over
  `LocalServer` (refresh rotates the token, sign-out); the token middleware
  matching the server middleware's wire format; a short secret refused.
- `apps/auth-server/tests/device_and_accounts.rs`: the device code approved
  and refused; several accounts and switching.
- `apps/auth-server/tests/http_surface.rs`: the OIDC provider, pages, CORS.
- `apps/auth-server/tests/mock_oauth.rs` and `apps/auth-server/tests/social.rs`:
  social sign-in against `apps/mock-oauth`.

## Status

Done in architect and deployed. Not yet: remote token introspection (until
then Task's `task:apps/server/src/central_auth.rs` `CentralAuth` is the only
one); tokens others can verify offline (HS256 with no published keys); a key
ring instead of one secret; revocation reaching offline peers.

- **Task** runs the engine on its server (`task:apps/server/Cargo.toml`), keeps
  sessions in a `FileTokenStore`, signs in with `auth_client::oidc`
  (`task:crates/ui/src/central_login.rs`) and uses `DeviceService`. Still to
  adopt: `TokenStoreMiddleware` and `refresh_if_needed`.
- **Studio** uses only `auth-client`: the device flow
  (`studio:apps/session/daw/src/task_account.rs`) and OIDC with PKCE
  (`studio:features/signal/tone3000/src/account/session.rs`).
