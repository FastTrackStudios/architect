+++
title = "Email verification"
description = "Rules for email verification tokens and verified state."
weight = 50
+++

# Email Verification

r[auth.verify.token-random]
Email verification values MUST be generated from cryptographically
secure randomness.

r[auth.verify.token-hash]
Only a hash of the verification value MUST be stored.

r[auth.verify.expiry]
Verification values MUST expire and expired values MUST fail without
marking an email verified.

r[auth.verify.single-use]
Verification values MUST be single-use.

r[auth.verify.identifier]
Verification records MUST include an identifier that scopes the value to
the intended user and purpose.

r[auth.verify.success]
Successful email verification MUST set `AuthUser.email_verified` to
true for the intended user.

r[auth.verify.resend-throttle]
Verification resend commands SHOULD be rate-limited by identifier to
avoid email abuse.

r[auth.verify.change-email]
Changing a user's email MUST clear verified state unless the new email
is proven through a trusted identity provider or verification flow.

r[auth.verify.plugin-descriptor]
Email verification MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.verify.plugin-routes]
The email verification plugin descriptor MUST reference generated
command metadata for request-email-verification and verify-email
transport/OpenAPI exposure.
