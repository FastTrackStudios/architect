---
"auth" = "minor"
"auth-server" = "minor"
---

Security: `/oauth2/linked-token` refuses an access token whose sign-in was revoked, as userinfo does (`ArchitectAuth::verify_signed_in_jwt`, what every endpoint taking an access token uses). Revoking a sign-in already revoked, or reusing its rotated refresh token again, stores nothing new, and writing a revocation mark deletes expired verifications (new required `AuthStorage::delete_expired_verifications`; a custom storage deletes rows with `expires_at <= now`). Sign-in ids are tagged with HMAC-SHA256 (`crypto::hmac_token`). An unknown key in an `AUTH_OIDC_CLIENTS` or `AUTH_OIDC_CLIENTS_EXTRA` entry is ignored with a start-up warning naming the client and the key; a near-miss of a key that matters (any key the issuer reads, such as `redirect_uris`, `first_party` or `rotate_refresh_tokens`, or a later issuer's session key: the same apart from case, `-` and `_`, or within two edits, such as `first_pary` or `redirect_uri`) is an error at start-up.
