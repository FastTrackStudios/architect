---
"auth" = "minor"
"auth-client" = "minor"
"auth-server" = "minor"
---

Security: the issuer revokes a sign-in at `/oauth2/revoke` (RFC 7009, `ArchitectAuth::revoke_oidc_token`, `RevokeOidcToken`; `auth_client::oidc::revoke_request_body`, `REVOKE_PATH`; advertised as `revocation_endpoint`) with one revocation mark on the sign-in (the `grant` every token of it carries) that every use checks: its refresh tokens mint nothing, its access tokens are refused at `whoami`, `list_organizations` and `/oauth2/userinfo`, and the issuer session stays. A public client registered with `rotate_refresh_tokens` (new `OidcClientConfig` field, `"rotate_refresh_tokens"` in `AUTH_OIDC_CLIENTS`, default `false`) gets single-use refresh tokens: one conditional write consumes a token (new required `AuthStorage::swap_verification_value`; a custom storage implements it as a compare-and-swap), a reuse within 10 s answers the same successor, a later one revokes the sign-in. Other clients keep reusable refresh tokens. OIDC tokens carry `token_use` (`access`/`id`) and `grant`. `issue_jwt` refuses the claims `client_id`, `token_use` and `grant` and never outlives its session; a client named like the JWT audience is a `ConfigError::ClientIsJwtAudience`.
