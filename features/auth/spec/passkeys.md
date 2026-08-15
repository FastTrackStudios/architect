+++
title = "Passkeys and WebAuthn"
description = "Rules for passkey registration and authentication."
weight = 90
+++

# Passkeys and WebAuthn

r[auth.passkey.challenge-random]
WebAuthn registration and authentication challenges MUST be generated
from cryptographically secure randomness.

r[auth.passkey.challenge-expiry]
WebAuthn challenges MUST expire and expired challenges MUST fail.

r[auth.passkey.rp-origin]
Passkey verification MUST enforce the configured relying party ID and
allowed origins.

r[auth.passkey.credential-unique]
`credential_id` MUST identify at most one stored passkey.

r[auth.passkey.user-match]
Registration MUST bind the credential to the authenticated user who
started registration.

r[auth.passkey.counter]
Authentication MUST update and validate the authenticator counter when
the authenticator provides one.

r[auth.passkey.delete-last-credential]
Deleting a passkey MUST be rejected when it would leave the user without
any usable sign-in credential, unless an admin recovery policy allows it.

r[auth.passkey.transports]
Authenticator transports SHOULD be persisted when supplied by the
client.

r[auth.passkey.list]
Users MUST be able to list only their own registered passkeys through an
authenticated session.

r[auth.passkey.plugin-descriptor]
The passkey plugin MUST declare its Better Auth lineage, session and
verification-token dependencies, and capabilities for registration,
authentication, credential management, relying-party validation, challenge
lifecycle, and authenticator counters.

r[auth.passkey.plugin-routes]
The passkey plugin routes MUST be generated from the shared auth command
catalog so Rust, Axum, Vox, and OpenAPI expose the same operation ids and
session requirements.
