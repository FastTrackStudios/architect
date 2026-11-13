# fts-auth: one account across the FTS apps

Source: the STA-219 architecture report and the `architect` wiki page `fts-auth`. Versions rot: read fts-auth's `Cargo.toml` for the architect tag it pins.

## What it is

- [FastTrackStudios/fts-auth](https://github.com/FastTrackStudios/fts-auth) is only the **deployment shell**: a short `main.rs`, a pinned architect tag, a Helm chart, a nix image build and a deploy workflow. The server itself is architect's `apps/auth-server` on the `architect-auth` engine (`references/auth-permissions.md`). Engine changes go upstream into architect.
- Issuer `auth.fasttrackstudio.app`: one Postgres (CNPG `pg-main`), one account per person. Argo CD app `fts-auth`, namespace `fts-auth`, 2 replicas. The image is built on the self-hosted `auth` runner on THEBATTLESHIP. Secret names (never read the values): `fts-auth-secrets`, `fts-auth-pg`. Secret changes go through Keeper (`secrets-handling`).

## How an app integrates

- An app is a **public OIDC client**: authorization code + PKCE, `offline_access` for a 7-day rotating refresh token. CLIs and headless windows use the **device-code** flow.
- The app server turns the bearer into a person by calling the issuer (`/auth/session` or `/oauth2/userinfo`), then applies **its own** membership fence and permit table (`architect-permissions`). Orgs and roles (owner/admin/member plus custom roles) live at the issuer; what a role may *do* inside an app lives in that app.
- **The issuer's own access token** (an OIDC access JWT) is taken in place of a session at `whoami` and `list_organizations` only, and only from a first-party client (registered, enabled, `skip_consent`) with `openid` and the default audience (`features/auth/auth/src/flows.rs` `access_token_caller`). That is how a node's org lane serves the web app (client `architect`, `issue_sessions: false`): the node relays the bearer to the issuer and never verifies a JWT (HS256 is a shared secret). Refresh, issue, sign-out and the device calls take a session alone (`session_caller`).
- **Offline:** tokens are HS256 (a shared secret only the issuer holds) with an empty JWKS, so no app, client or peer can validate them offline. Local-first works because each native install runs its own node with a local account, using the issuer only to sign in, list orgs and keep the device address book (`features/auth/auth-proto/src/device.rs` `DeviceService`). Peers then authenticate each other by **iroh endpoint key**, not by fts-auth tokens.
- Task is the only app wired end to end, with its own pass (task#404 moves it onto architect). Wire Signal or Session with `architect_devices::account::pass` / `drive` (`features/devices/src/account.rs`), not a copy of Task's.

## Known gaps (don't trip on them)

- The Architect app (`apps/ui/src/accounts.rs` `use_app_accounts`) has no OIDC client at the issuer: starcommand's fts-auth module (modules/services/fts-auth/fts-auth.nix) registers one per `appDomains` entry (task, studio, session, signal, keyflow, ignition). Until an `architect` client exists (with `app.fasttrackstudio.architect://auth/callback` and a loopback redirect), it signs in by browser code (`/auth/device/*`) or password, which hold issuer sessions, not OIDC access JWTs: no `offline_access` refresh token, and `/oauth2/linked-token` refuses them (as for Session).

- The issuer has no personal org per account (`apps/auth-server/src/dev.rs` seeds team orgs only, and sign-up creates none), so `architect_orgs::from_bundles` finds no `personal` and the Architect app puts `architect_accounts::device_org` (id `me`, this device only) first. When the issuer gives each account an org whose slug is its user id, that one takes its place.
- `OrganizationService::set_active_organization` is all the issuer keeps of the current org: per session, not per account or device, and nothing reads it back for a client (`AuthService::current_session` carries it, `list_organizations` does not). The app keeps the current org itself (`accounts.json`) and tells the issuer best effort (`Accounts::switch_org`).
- No org-scoped vox endpoint is advertised: a client builds `/org/{slug}/vox` itself, and nothing at the issuer says which server hosts an org (Task's well-known org list, iroh endpoint id and `disabled_plugins` have no issuer counterpart yet).
- Check that Signal and Session pin an architect tag with `DeviceService` before wiring the account sync pass; neither runs it yet.
- Signal's default redirect `http://localhost:4040/account/callback` may not be registered at the issuer: sign-in needs the env override, or the redirect added in the cluster module.
- Session's device-code session token can't read linked provider tokens: `/oauth2/linked-token` accepts only OIDC access JWTs.
- Revocation doesn't reach offline peers: `forget_device` only stops advertising the device; each peer drops it on its own next account pass (`Admission::set_account`), which also closes its connections there.
- One `auth-secret` hashes sessions, signs JWTs and encrypts stored provider tokens, with no key ring. Rotating it signs everyone out and forces them to re-link. Treat a rotation as a Captain yes/no.
- Linked agents aren't consumed by any app yet. Task's bots are a separate system.
- Remote token introspection (I34) is not in architect.
