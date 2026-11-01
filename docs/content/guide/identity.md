+++
title = "Sign-in, permissions and organizations"
description = "Sign a person in against the one identity server, keep the session and register the device; gate every service call with PermissionsGate; and make the organization the scope of everything, on the client and confined on the server."
weight = 120
+++

Every FastTrackStudio app signs in to one identity server (`apps/auth-server`,
deployed as fts-auth) and reads the person's organizations from it, so an
org is the same org in every app. In this chapter you will:

1. sign a person in, keep the session, and put its token on every call;
2. register the device on the person's account;
3. gate every service on the server with `PermissionsGate` and permit
   tables;
4. give the client the person's orgs, the window's current org and the
   switcher, fetched live;
5. key every layer's data by `OrgId::path`, and confine each server lane to
   its org;
6. read across every org for an "All organizations" view.

The crates: the auth family under `features/auth` (there is no
`architect::auth` re-export; depend on them directly),
`architect-permissions` (`features/permissions`), architect's
`crates/architect/src/permissions_gate.rs`, and `architect-orgs`
(`features/orgs`).

## 1. Sign in and keep the session

`auth-client` (`features/auth/auth-client`) is the client kit: wasm-clean,
protocol only (your app posts with its own HTTP client).

- **A desktop or web app** signs in with OIDC and PKCE:
  `oidc::authorize_url`, `Pkce::from_entropy`, `oidc::token_request_body`,
  then `oidc::token_response_from(body)?.into_session(now)`, which keeps the
  refresh token.
- **A headless tool** (a CLI, an agent) uses the RFC 8628 device flow:
  `device::start_body`, `device::parse_start`, `device::poll_body`,
  `device::parse_poll`.
- **An app calling `AuthService` directly** turns its sign-in answer into a
  session with `architect_auth::client::stored_session(&bundle)`.

Keep the session in a `TokenStore` and attach it to every vox client with
`TokenStoreMiddleware`, which reads the store on every call, so a fresh
sign-in is picked up without rebuilding clients:

```rust
use auth_client::{FileTokenStore, TokenStoreMiddleware};
use architect_auth::client::stored_session;

let store = Arc::new(FileTokenStore::new(dirs.data_dir().join("session.json"))); // 0600
store.save(&stored_session(&bundle))?;                                          // after sign-in
let tasks = tasks_client.with_middleware(TokenStoreMiddleware::new(store.clone()));
store.clear()?;                                                                 // sign out
```

`MemoryTokenStore` is the wasm and test store. To stay signed in past the
access token's hour, call `oidc::refresh_if_needed(&store, client_id, now, send)`
before a call and `oidc::refresh_now` after a 401; a `Refused` answer means
sign in again. Refresh tokens rotate, so run one refresh per store at a time.

## 2. Register the device

The account keeps a device address book (`features/auth/auth-proto/src/device.rs`
`DeviceService`: `register_device`, `list_devices`, `forget_device`). Every
FTS app on one machine registers under the same device id, derived from what
the platform gives every app of one developer alike
(`features/auth/auth-client/src/this_device.rs` `install_id`, `device_id`),
so the account lists an iPad once however many apps run on it:

```rust
let registration = DeviceRegistration { /* device_id, name, platform, endpoint_id */ };
ensure_registered(&devices_client, &token, registration).await?;
```

`ensure_registered` only adds a row when nothing names the device yet,
unless the app has a node of its own (an iroh `endpoint_id`), which it
always keeps current. The answer is a `DeviceDirectory`; its
`peer_endpoints(this_device)` is what the
[documents chapter](@/guide/documents.md)'s `Devices` admits for
device-to-device sync.

## 3. Permissions: gate every call

On the server every service call passes a `PermissionsGate`: it resolves
the caller (`Principal`) from the bearer token, asks a `PermissionEngine`
whether that principal may do the method's action on its resource, and
replies `permission denied: …` in the method's own response shape when not.
It fails closed: a method a table omits is denied, and so is a service with
no table (`UnlistedPolicy::Deny`, the default).

Write one permit table per service:

```rust
const NOTES: ServicePermits = ServicePermits {
    service: "notes",
    methods: &[
        MethodPermit::new("list", Action::READ, "notes/**"),
        MethodPermit::new("save", Action::WRITE, "notes/{id}"),
    ],
};
```

Then build the gate once and install it:

```rust
let gate = PermissionsGate::new(engine, Arc::new(SessionIdentityResolver::new(auth.clone())))
    .permit(notes_service_descriptor(), &NOTES);
let router = router.with_permissions(gate);
// or, per connection with its bearer: PermissionsGate::wrap_shared_with_bearer(Arc::new(gate), router, bearer)
```

- **The engine.** `RoleEngine` (roles to rules), `ScopeEngine` (public or
  link scopes), `CompositeEngine` (several in order). An org's server uses
  `ConfinedEngine` (step 5).
- **Coverage.** `gate.coverage(&router).is_complete()` in a test proves every
  method of every mounted service is in a table and no table names a method
  that does not exist.
- **The caller in a handler.** `architect::permissions_gate::caller()` is
  the principal the gate resolved; it is task-local, so work spawned onto
  another task sees `None` and must refuse.
- **The same engine for actions.** The gate covers service calls, not the
  Action List's in-process runs. Give the list the same engine
  (`list.set_authorizer(engine)`) so a model over MCP is checked by the
  same rules.
- **What the client may do.** `PermissionsService` (`features/permissions/proto`)
  is the affordance oracle: serve it from the gate's own engine and resolver
  (`Permissions::new(gate.engine(), gate.identity_resolver())`) so the UI can
  dim what the person cannot do, and the answer cannot disagree with
  enforcement.

<!-- media: action-disabled-by-role — a viewer's palette and ⋯ menu with Invite people dimmed and the reason shown -->

While migrating an existing server, `observe_only(true)` audits what would
be denied and lets everything through.

## 4. The person's orgs on the client

`architect-orgs`' default build is the model: `Org` (id, name, the
person's `Role`), `Orgs` (the list, the window's `Selection`, cached on the
device), and `Capabilities`, the app's own meaning of each role. Roles are
the org's; what a role lets a member do in this app is the app's:

```rust
let caps = Capabilities::new()
    .allow(&Role::Viewer, ["notes.read"])
    .allow(&Role::Member, ["notes.write", "files.write"])
    .allow(&Role::Admin, ["org.invite"])
    .inherit();                                   // each role gets the ones below it
```

Feature `ui` puts them in context and keeps them live:

```rust
let org = architect_orgs::use_orgs_provider(
    || cached_orgs(),                             // from the device's cache: works offline
    caps,
    Some(Callback::new(move |orgs: Orgs| keep_cache(orgs))),
);
architect_orgs::use_orgs_live(org, connection, |client| async move {
    architect_orgs::fetch(&client, &session).await.map_err(|e| e.to_string())
});
```

`use_orgs_live` fetches the person's orgs (`OrganizationService::list_organizations`
with the stored session) when the connection comes up and again on every
reconnect (`Connection::generation`). A failed fetch keeps the cached list,
and `OrgHandle::stale` says why.

Anywhere below:

```rust
let org = architect_orgs::use_org();                       // Option<OrgHandle>
let current = org.and_then(|o| o.current());               // None in All
let can_invite = org.is_some_and(|o| o.can("org.invite"));
```

The shell's switcher is `org.switcher(Some("All organizations"))`, a
`Workspaces` to pass to `AppShell { workspaces }`
([UI chapter](@/guide/ui.md)); picking one calls `OrgHandle::switch`.

<!-- media: org-switcher-all — the organization switcher: picking an org, then All organizations with items tagged by org -->

## 5. The org as the scope of everything

Every layer keys an org's data the same way, `OrgId::path`, so files,
documents, settings and presence agree without knowing each other:

| Layer | Keyed by |
|---|---|
| Files | `org.path("files/…")` |
| Documents | the catalogue `org.doc_id("catalogue")`; a document's blob under `org.path("docs/<id>")` |
| Settings | the org's own under `org.path("settings")` |
| Presence | the org's roster is the document `org.doc_id("presence")` |
| Permissions | resources `org/<id>/…` |

`path` gives `org/<id>/<rest>`; `doc_id` a v5 UUID of it, the same in every
app and on every device.

### Server-side confinement

An org's server serves each org on its own lane (`/org/{slug}/vox`) and
confines every call on it (`architect-orgs` feature `server`,
`features/orgs/src/server.rs`):

1. `Members::new()`, shared by the next two.
2. Wrap your identity resolver: `MembersResolver::new(sessions, IdentityServer(orgs_client), members.clone())`
   learns each session's orgs from the identity server, at most once a
   minute per session.
3. Use `ConfinedEngine::new(members).role(&Role::Member, vec![Rule::new("**", &["read", "write"])])`
   as the gate's engine (`.over(engine)` adds yours after it). It refuses a
   caller who is not a member of the call's org, any resource in another
   org, and what the caller's role there does not allow.
4. Per connection: `confine(org_id, PermissionsGate::wrap_shared_with_bearer(gate, router, bearer))`.
5. Handlers read `current_org()` and key their data by it; work they spawn
   runs `in_org(org, fut)`.

`confine` only says which org a call is in; without `ConfinedEngine` in the
gate nothing refuses a non-member. `features/orgs/tests/server.rs` is the
whole loop over a real vox link.

## 6. All organizations

In All, a read covers every org the person is in; a write needs one:

```rust
let scope = org.scope();                         // the window's org, or every org in All
let all = read_all(&scope, |id| async move { notes_in(&id).await }).await;
for item in all.items { /* Scoped { org, item } */ }
for (org, why) in all.failed { /* an offline org hides none of the others */ }
let target = scope.writes_to();                  // None in All: ask which org first
```

⌘O across orgs is `scoped_opener` (feature `commands`): it asks the
window's org, or every org naming each item's org, and opens an item in the
org it came from. On the server, a lane in no org serves these reads: its
resources name their org, and a service reads across `Members::scope_of`
the caller.

## Where Task and Studio are

Sign-in, devices, the gate, the org model, live fetch and confinement are
architect-side. Task still keeps its own `OrgSelection` and confines its
own lanes (with `UnlistedPolicy::Allow` and enforcement behind
`TASK_ENFORCE_PERMISSIONS`); Studio has its spaces. Moving both onto
`architect-orgs`, and files, documents, settings and presence adopting
the scope as each lands, is still to do. Which org's settings apply, and
an admin's page for them, are not built yet.

## Where to read more

- [The auth feature](@/architecture/auth.md): the RPC surface, the server,
  the client session kit.
- [Organizations](@/architecture/organizations.md): the one identity
  server, the scope, confinement.
- `skills/architect/references/auth-permissions.md` (gate decisions,
  principals, pitfalls) and `skills/architect/references/fts-auth.md` (the
  deployed issuer).
