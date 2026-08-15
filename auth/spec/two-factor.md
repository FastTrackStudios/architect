+++
title = "Two-factor authentication"
description = "Rules for TOTP and backup-code based second factors."
weight = 80
+++

# Two-factor Authentication

r[auth.twofactor.enable-requires-session]
Enabling two-factor authentication MUST require an authenticated user
session.

r[auth.twofactor.secret-encryption]
TOTP secrets MUST be encrypted before storage.

r[auth.twofactor.confirm-before-enabled]
Two-factor authentication MUST NOT be marked enabled until the user
successfully proves possession of the newly generated factor.

r[auth.twofactor.signin-required]
Users with two-factor enabled MUST complete a second-factor challenge
before a normal active session is issued.

r[auth.twofactor.backup-codes-hash]
Backup codes MUST be stored only as hashes.

r[auth.twofactor.backup-codes-single-use]
Backup codes MUST be single-use.

r[auth.twofactor.disable-requires-proof]
Disabling two-factor authentication MUST require either a valid second
factor, a recovery code, or an authorized admin recovery flow.

r[auth.twofactor.rate-limit]
Second-factor verification attempts SHOULD be rate-limited by user and
challenge.

r[auth.twofactor.plugin-descriptor]
The two-factor plugin MUST declare its Better Auth lineage, session
dependency, and capabilities for TOTP setup, pending-session verification,
backup codes, rate limiting, and encrypted secrets.

r[auth.twofactor.plugin-routes]
The two-factor plugin routes MUST be generated from the shared auth command
catalog so Rust, Axum, Vox, and OpenAPI expose the same operation ids and
session requirements.
