+++
title = "Permissions"
description = "Principal, resource, action: permit tables per service, PermissionsGate on every mounted router, observe-only for migration, and the same engine for Action List runs."
weight = 31
+++

## What it is and when to use it

The model is a `Principal` (a user, a guest, a service, a host device, or
anonymous) doing an `Action` (read, write, comment, admin, download, run) on
a `Resource` (a path such as `org/fts/files/**`), and a `PermissionEngine`
answering Allow or Deny. Each service has a `ServicePermits` table naming,
per method, the action and resource it needs. `PermissionsGate` checks every
vox call against those tables before the method runs, and denies what it
cannot place: a method a table leaves out, or a service with no table
(unless told otherwise). `observe_only` audits what it would deny and lets it
through, for turning the gate on in a running app.

Mount every service behind the gate and give the Action List the same engine
(`set_authorizer`): the router gate does not cover in-process action runs.
There is no architecture page for the gate; the reference is
`skills/architect/references/auth-permissions.md`. Signed, expiring grants
are a different tool: [Capability tokens](@/architecture/capability.md).

## A minimal example

`architect-permissions`; the gate is `architect::permissions_gate` (feature
`vox`, native). From `features/auth/tests/native/src/permissions_gate_tests.rs`:

```rust
use architect::permissions_gate::{PermissionsGate, UnlistedPolicy};
use architect_permissions::{MethodPermit, RoleEngine, ServicePermits};
use auth::identity::SessionIdentityResolver;

const PROBE_PERMITS: ServicePermits = ServicePermits {
    service: "gate-probe",
    methods: &[
        MethodPermit::new("read_thing", "read", "probe/**"),
        MethodPermit::new("write_thing", "write", "probe/**"),
    ],
};

let mut roles = RoleEngine::new();
roles.set_member(alice.user.id.to_string(), "member");
let gate = PermissionsGate::new(Arc::new(roles), Arc::new(SessionIdentityResolver::new(auth.clone())))
    .unlisted(UnlistedPolicy::Deny)          // the default
    .observe_only(!enforce)
    .permit(gate_probe_service_descriptor(), &PROBE_PERMITS);
assert!(gate.coverage(&router).is_complete());
let gated = router.with_permissions(gate);
```

Per connection, with the connection's bearer:
`PermissionsGate::wrap_shared_with_bearer(Arc::new(gate), router, bearer)`.
Inside a handler, `architect::permissions_gate::caller()` is the principal
the gate resolved. For actions: `list.set_authorizer(engine)` and run with
`run_json_as` or `run_json_async_as`.

## Code tour

1. `features/permissions/src/lib.rs` `Principal`, `Resource`, `Action`,
   `Decision`: the model.
2. `features/permissions/src/lib.rs` `PermissionEngine` (`check`, `authorize`)
   and `IdentityResolver`: the two traits a gate needs.
3. `features/permissions/src/lib.rs` `RoleEngine`, `ScopeEngine`,
   `CompositeEngine`, `AllowAll`, `Rule`: engines to start from.
4. `features/permissions/src/lib.rs` `MethodPermit`, `ServicePermits`,
   `AuditSink`, `TracingAudit`: the tables and the audit trail.
5. `features/auth/auth/src/identity.rs` `SessionIdentityResolver`: a bearer to
   a user or anonymous.
6. `crates/architect/src/permissions_gate.rs` `PermissionsGate` (`permit`,
   `unlisted`, `observe_only`, `coverage`, `wrap_shared_with_bearer`),
   `PermissionedRouter`, `caller`, `GateCoverage`.
7. `crates/architect/src/action_list.rs` `ActionList::set_authorizer`,
   `run_json_as`, `infos_for`, `ActionCaller`: the same engine for actions.
8. `features/permissions/proto/src/lib.rs` `PermissionsService`: an advisory
   "can I?" service a UI asks before showing a control.

A call reaches `PermissionedRouter`, which takes the bearer from the call's
metadata or else the connection's. It maps the method id to its permit,
resolves the principal once, and asks the engine about the permit's resource
(`{arg}` placeholders widen to `**`). On Allow the method runs with the
principal set, so `caller()` sees it. On Deny the reply is `permission
denied` in the method's own response shape, unless `observe_only` is on, in
which case the denial is audited and the call goes through.

## Tests to read

- `features/auth/tests/native/src/permissions_gate_tests.rs`: roles enforced
  over validated sessions; a scope engine gating by prefix; observe-only
  letting denials through; a handler seeing the resolved caller; an untabled
  service denied by default and served when allowed; coverage naming every gap.
- Unit tests in `crates/architect/src/action_list.rs`: the authorizer decides
  who runs what; a caller sees only what it may run.
- `features/scripting/tests/scripts.rs`: a script runs with its caller's permissions.
- `features/orgs/tests/server.rs`: the gate with an org-confined engine.

## Status

Done in architect. Limits: the gate checks the method's coarse resource,
not each argument; a malformed token is treated as anonymous; revocation lags
by the resolver's cache (30 s); the advisory service checks as anonymous;
the gate and the Action List are two planes until the app wires
`set_authorizer`.

- **Task** gates its server (`task:apps/server/src/lib.rs`, tables in
  `task:apps/server/src/permits.rs`) with unlisted services allowed and
  enforcement behind `TASK_ENFORCE_PERMISSIONS=1`. Still to do: deny unlisted,
  enforce by default, `set_authorizer` on its Action List.
- **Studio** depends on `architect-permissions` but mounts no gate yet.
