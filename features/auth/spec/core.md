+++
title = "Core auth model"
description = "Rules shared by every architect-auth capability."
weight = 10
+++

# Core Auth Model

r[auth.core.entities.single-source]
The entity shape for users, sessions, accounts, organizations, members,
invitations, verifications, two-factor records, API keys, and passkeys
MUST be declared once in `auth-proto` and consumed by all runtime,
storage, transport, and test crates.

r[auth.core.server-authoritative]
Auth state MUST be server-authoritative. Clients MAY submit commands and
opaque tokens, but clients MUST NOT be trusted as the source of truth for
identity, roles, session validity, verification state, or credential
state.

r[auth.core.no-plaintext-secrets]
Passwords, session tokens, verification values, backup codes, API keys,
OAuth tokens, TOTP secrets, and passkey private material MUST NOT be
stored in plaintext.

r[auth.core.secret-aead]
Runtime secret envelopes MUST use reviewed AEAD encryption with
authenticated metadata instead of ad hoc stream or tag construction.

r[auth.core.secret-key-ids]
Runtime secret envelopes MUST include key identifiers so encrypted
values can be traced to a configured key without trial decryption.

r[auth.core.secret-rotation]
Runtime secret decryption MUST support a current encryption key plus
fallback decryption keys for rotation.

r[auth.core.secret-legacy-decrypt]
Runtime secret decryption SHOULD keep a bounded legacy decrypt path for
existing encrypted values until a migration rewrites them.

r[auth.core.secret-minimum]
The runtime config MUST reject auth secrets shorter than 32 bytes.

r[auth.core.errors-stable]
Public command failures MUST map to stable `AuthFlowError` variants or
typed feature-specific errors that can be transported over Vox and HTTP
without exposing sensitive internals.

r[auth.core.timestamps]
Server-created records MUST use the server clock for `created_at`,
`updated_at`, expiry, revocation, and ban timestamps.

r[auth.core.metadata-json]
Extensible metadata fields MUST be stored as JSON strings until a typed
metadata model is introduced. Runtime code MUST validate metadata is
well-formed JSON before persisting user-provided values.

r[auth.core.feature-flags]
Optional capabilities MUST be gated behind Cargo features or builder
configuration without changing the core entity model.

r[auth.user.plugin-descriptor]
User management MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.user.plugin-routes]
The user management plugin descriptor MUST reference generated command
metadata for user-management transport/OpenAPI exposure.

r[auth.user.delete]
An authenticated caller MUST be able to delete their own user record.
Deletion MUST revoke or remove active sessions and credentials owned by
that user.

r[auth.account.plugin-descriptor]
Account management MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.account.plugin-routes]
The account management plugin descriptor MUST reference generated command
metadata for account-management transport/OpenAPI exposure.

r[auth.account.list]
An authenticated caller MUST be able to list only their own linked
accounts and credential records.
