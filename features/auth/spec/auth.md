+++
title = "architect-auth"
description = "Authentication and authorization rebuilt around Architect contracts."
+++

# architect-auth

The auth feature owns identity, sessions, credentials, linked provider
accounts, organizations, invitations, two-factor state, API keys, and
passkeys.

Rules:

- Auth state is server-authoritative.
- Public contracts are Architect entities and service traits.
- Secrets are never stored in plaintext.
- Session tokens and verification values are stored as hashes.
- OAuth tokens and TOTP secrets are stored encrypted once the encryption
  layer lands.
- Runtime flows are typed commands that can be exposed through Axum,
  Vox, or another transport without changing domain code.

