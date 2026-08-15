+++
title = "Bearer tokens"
description = "Rules for Authorization bearer token authentication."
weight = 97
+++

# Bearer Tokens

r[auth.bearer.parse]
Bearer authentication MUST parse the `Authorization` header, require the
`Bearer` scheme, reject malformed token values, and treat a missing header as
invalid credentials.

r[auth.bearer.session]
Bearer authentication MUST accept active session tokens.

r[auth.bearer.api-key]
Bearer authentication MUST accept enabled, unexpired API keys when session
authentication does not match.

r[auth.bearer.errors]
Missing, malformed, expired, revoked, and disabled bearer tokens MUST map to
the stable auth error taxonomy.

r[auth.bearer.plugin-descriptor]
The bearer plugin MUST declare its Better Auth lineage, session/API-key
dependencies, and capabilities for Authorization headers, Axum middleware,
Vox metadata, and stable errors.

r[auth.bearer.plugin-routes]
The bearer plugin routes MUST be generated from the shared auth command
catalog so Rust, Axum, Vox, and OpenAPI expose the same operation ids and
session requirements.
