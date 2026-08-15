+++
title = "JWT"
description = "Rules for JWT issuing, verification, and rotation."
weight = 101
+++

# JWT

r[auth.jwt.sign]
JWT issuing MUST sign tokens with the configured active key and include a key
id header.

r[auth.jwt.verify]
JWT verification MUST validate the signature with the matching configured
key.

r[auth.jwt.claims]
JWT claims MUST include stable issuer, audience, subject user id, session id,
issued-at, expiry, and optional extra claims.

r[auth.jwt.expiry]
Expired JWTs MUST fail verification.

r[auth.jwt.issuer-audience]
JWT verification MUST enforce configured issuer and requested or default
audience.

r[auth.jwt.revoked-session]
Session-backed JWT verification MUST fail when the backing session is
revoked or expired.

r[auth.jwt.rotation]
JWT key rotation MUST sign with the active key and verify with configured
fallback keys by key id.

r[auth.jwt.jwks]
JWT key-set metadata MUST expose key ids, algorithms, and active status
without exposing symmetric signing secrets.

r[auth.jwt.plugin-descriptor]
The JWT plugin MUST declare its Better Auth lineage, session dependency, and
capabilities for signing, verification, issuer/audience enforcement, stable
claims, session backing, key rotation, and key-set metadata.

r[auth.jwt.plugin-routes]
The JWT plugin routes MUST be generated from the shared auth command catalog
so Rust, Axum, Vox, and OpenAPI expose the same operation ids and session
requirements.
