---
title = "One-time token"
description = "Rules for service-defined single-use tokens."
---

# One-Time Token

r[auth.ott.create]
One-time token generation MUST require an active session and store only a
server-side token verifier bound to that session.

r[auth.ott.consume]
One-time token verification MUST consume the token and return the bound
session when valid.

r[auth.ott.expire]
Expired one-time tokens MUST be rejected.

r[auth.ott.replay]
Previously consumed one-time tokens MUST be rejected.

r[auth.ott.revoke]
One-time tokens MUST be revocable before use.

r[auth.ott.scope]
One-time tokens MAY carry a service-defined scope and verification MUST
reject mismatched required scopes.

r[auth.ott.metadata]
One-time tokens MAY carry service-defined JSON metadata that is returned
after successful verification.

r[auth.ott.plugin-descriptor]
One-time token behavior MUST be represented by an Architect auth plugin
descriptor with upstream Better Auth parity, dependencies, and capability
metadata.

r[auth.ott.plugin-routes]
The one-time token plugin descriptor MUST reference generated command
metadata so Rust, Axum, Vox, and OpenAPI expose the same operation ids.
