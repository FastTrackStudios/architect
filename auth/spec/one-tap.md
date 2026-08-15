---
title = "One Tap"
description = "Rules for Google One Tap sign-in."
---

# One Tap

r[auth.onetap.token-validation]
One Tap callback MUST validate the provider ID token issuer, audience,
signature, and expiration before trusting claims.

r[auth.onetap.existing-user]
One Tap callback MUST sign in an existing linked provider account. If a
matching email user exists without a provider account, implicit linking
MUST require both local and provider email verification.

r[auth.onetap.new-user]
One Tap callback MAY create a new user and Google account when signup is
enabled and no matching user exists.

r[auth.onetap.disabled-signup]
One Tap callback MUST reject new users when signup is disabled.

r[auth.onetap.session]
Successful One Tap callback MUST create a server-side session and return
the session token with the user and session.

r[auth.onetap.plugin-descriptor]
One Tap behavior MUST be represented by an Architect auth plugin descriptor
with upstream Better Auth parity, dependencies, and capability metadata.

r[auth.onetap.plugin-routes]
The One Tap plugin descriptor MUST reference generated command metadata so
Rust, Axum, Vox, and OpenAPI expose the same operation ids.
