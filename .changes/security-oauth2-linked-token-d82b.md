---
"auth" = "minor"
"auth-server" = "minor"
---

Security: `/oauth2/linked-token` refuses an access token whose sign-in was revoked, as userinfo does (`ArchitectAuth::verify_signed_in_jwt`, what every endpoint taking an access token uses). Revoking a sign-in already revoked, or reusing its rotated refresh token again, stores nothing new, and writing a revocation mark deletes expired verifications (new required `AuthStorage::delete_expired_verifications`; a custom storage deletes rows with `expires_at <= now`). Sign-in ids are tagged with HMAC-SHA256 (`crypto::hmac_token`). An unknown key in an `AUTH_OIDC_CLIENTS` entry (a misspelled `first_party`) is an error at start-up.
