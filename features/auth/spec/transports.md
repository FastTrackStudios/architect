+++
title = "Transport integrations"
description = "Rules for Axum, Vox, OpenAPI, hooks, and command surfaces."
weight = 120
+++

# Transport Integrations

r[auth.transport.domain-first]
Runtime flows MUST be expressed as typed domain commands before being
adapted to Axum, Vox, OpenAPI, or any other transport.

r[auth.transport.axum-feature]
Axum integration MUST be optional behind the `axum` feature.

r[auth.transport.vox-schema]
Vox service traits MUST use the same domain command and response types
as non-Vox callers.

r[auth.transport.cookie-security]
HTTP session cookies MUST support secure, http-only, same-site, domain,
path, and max-age configuration.

r[auth.transport.openapi]
OpenAPI output SHOULD derive from the same route or command descriptors
used by the runtime, rather than a separately maintained schema.

r[auth.transport.command-metadata]
Auth transport metadata MUST have a single generated command catalog that
includes the plugin id, operation id, method, path, command type,
response type, and authentication requirement. Route descriptors and
OpenAPI output MUST be adapter views over this catalog.

r[auth.transport.hooks-typed]
Hooks and middleware MUST receive typed command context and typed
results, not stringly plugin payloads.

r[auth.transport.error-mapping]
Transport adapters MUST map internal errors to public responses without
leaking secrets, hashes, token existence, or stack traces.

r[auth.boundary.property-tests]
Untrusted auth boundaries MUST have property or fuzz-style tests covering
cookie/header token parsing, bearer extraction, URL trust checks,
permission JSON, plugin registry/OpenAPI generation, and invite/token
identifier parsing.

r[auth.boundary.fixtures]
Boundary regressions SHOULD add minimized fixtures when a property test
finds a stable failing input.
