# fts-auth: one account across the FTS apps

Source: the STA-219 architecture report and the `architect` wiki page `fts-auth`. Versions rot: read fts-auth's `Cargo.toml` for the architect tag it pins.

## What it is

- [FastTrackStudios/fts-auth](https://github.com/FastTrackStudios/fts-auth) is only the **deployment shell**: a short `main.rs`, a pinned architect tag, a Helm chart, a nix image build and a deploy workflow. The server itself is architect's `apps/auth-server` on the `architect-auth` engine (`references/auth-permissions.md`). Engine changes go upstream into architect.
- Issuer `auth.fasttrackstudio.app`: one Postgres (CNPG `pg-main`), one account per person. Argo CD app `fts-auth`, namespace `fts-auth`, 2 replicas. The image is built on the self-hosted `auth` runner on THEBATTLESHIP. Secret names (never read the values): `fts-auth-secrets`, `fts-auth-pg`. Secret changes go through Keeper (`secrets-handling`).

## How an app integrates

- An app is a **public OIDC client**: authorization code + PKCE, `offline_access` for a 7-day rotating refresh token. CLIs and headless windows use the **device-code** flow.
- The app server turns the bearer into a person by calling the issuer (`/auth/session` or `/oauth2/userinfo`), then applies **its own** membership fence and permit table (`architect-permissions`). Orgs and roles (owner/admin/member plus custom roles) live at the issuer; what a role may *do* inside an app lives in that app.
- **Offline:** tokens are HS256 (a shared secret only the issuer holds) with an empty JWKS, so no app, client or peer can validate them offline. Local-first works because each native install runs its own node with a local account, using the issuer only to sign in, list orgs and keep the device address book (`features/auth/auth-proto/src/device.rs` `DeviceService`). Peers then authenticate each other by **iroh endpoint key**, not by fts-auth tokens.
- Task is the only app wired end to end. Copy its account sync pass and `DeviceService` use when you wire Signal or Session.

## Known gaps (don't trip on them)

- Check that Signal and Session pin an architect tag with `DeviceService` before wiring the account sync pass; neither runs it yet.
- Signal's default redirect `http://localhost:4040/account/callback` may not be registered at the issuer: sign-in needs the env override, or the redirect added in the cluster module.
- Session's device-code session token can't read linked provider tokens: `/oauth2/linked-token` accepts only OIDC access JWTs.
- Revocation doesn't reach offline peers: `forget_device` only stops advertising the device.
- One `auth-secret` hashes sessions, signs JWTs and encrypts stored provider tokens, with no key ring. Rotating it signs everyone out and forces them to re-link. Treat a rotation as a Captain yes/no.
- Linked agents aren't consumed by any app yet. Task's bots are a separate system.
- Remote token introspection (I34) is not in architect.
