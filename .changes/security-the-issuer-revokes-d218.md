---
"auth" = "minor"
"auth-client" = "minor"
"auth-server" = "minor"
---

Security: the issuer revokes a sign-in at `/oauth2/revoke` (RFC 7009, `ArchitectAuth::revoke_oidc_token`, `RevokeOidcToken`; `auth_client::oidc::revoke_request_body`, `REVOKE_PATH`; advertised as `revocation_endpoint`): the grant of a refresh or access token ends, the issuer session stays. A public client's refresh token is single use (rotated on each refresh; a used one presented again past a few seconds revokes its grant). OIDC tokens carry `token_use` (`access`/`id`) and `grant`, and `whoami`/`list_organizations` take only an access token of a grant in force. `issue_jwt` refuses the claims `client_id`, `token_use` and `grant` and never outlives its session; a client named like the JWT audience is a `ConfigError::ClientIsJwtAudience`.
