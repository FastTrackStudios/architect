+++
title = "Email/password"
description = "Rules for email/password registration and sign-in."
weight = 20
+++

# Email/password

r[auth.email.signup.enabled]
When email/password is enabled, the runtime MUST expose a typed command
for creating a user with email and password.

r[auth.email.signup.disabled]
When email/password is disabled, email/password signup and sign-in
commands MUST fail without reading or writing credential data.

r[auth.email.email-normalization]
Email lookup MUST use a canonical form that is case-insensitive for the
domain and provider-compatible for the local part. The original display
email MAY be preserved separately if the entity model later adds it.

r[auth.email.email-unique]
Creating an email/password credential MUST reject duplicate canonical
emails.

r[auth.email.password-hash]
Passwords MUST be hashed with Argon2 or a stronger configured password
hashing strategy before storage.

r[auth.email.password-never-returned]
Password hashes and raw passwords MUST NOT be returned from public
commands, logs, route responses, or transport payloads.

r[auth.email.signin.invalid-generic]
Sign-in with an unknown email, missing password account, or wrong
password MUST return the same invalid-credentials error.

r[auth.email.signin.banned]
Sign-in for a banned user MUST fail while the ban is active.

r[auth.email.signin.verification-required]
If email verification is required, sign-in for an unverified user MUST
return a verification-required error before creating an active session.

r[auth.email.signin.success]
Successful sign-in MUST create an active session and return an
`AuthSessionBundle` containing the user, session, and raw session token.

r[auth.email.plugin-descriptor]
The email/password capability MUST be represented by an Architect auth
plugin descriptor with a stable id, upstream parity target, declared
dependencies, and capability names.

r[auth.email.plugin-routes]
The email/password plugin descriptor MUST own the signup and sign-in
route metadata used by transport and OpenAPI adapters.
