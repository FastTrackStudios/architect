---
title = "OIDC provider"
description = "Rules for OpenID Connect provider discovery, authorization code, token, and userinfo behavior."
---

# OIDC Provider

r[auth.oidc.discovery]
OIDC provider metadata MUST expose stable issuer, authorization, token,
userinfo, JWKS, registration, supported scope, response type, grant type,
client authentication method, signing algorithm, PKCE, and claim metadata.

r[auth.oidc.client-registration]
Client registration MUST support configured/trusted clients and MAY support
dynamic registration only when explicitly enabled. Registered redirect URIs
MUST be validated before issuing a client.

r[auth.oidc.authorization-code]
Authorization requests MUST validate the client, redirect URI, response
type, requested scopes, and current session before issuing a single-use
authorization code.

r[auth.oidc.pkce]
When PKCE is required, authorization requests MUST include an S256
challenge and token exchange MUST reject an incorrect verifier.

r[auth.oidc.consent]
Trusted clients MAY skip consent. Clients that require consent MUST produce
a verification-required result instead of silently issuing a code.

r[auth.oidc.token]
The token endpoint MUST exchange a valid authorization code for bearer
access and ID tokens and MUST bind the ID token audience to the client.

r[auth.oidc.refresh-token]
The token endpoint MUST issue refresh tokens only for `offline_access` and
MUST accept valid refresh tokens for access and ID token renewal.

r[auth.oidc.refresh-rotation]
A public client registered with `rotate_refresh_tokens` MUST get single-use
refresh tokens: each refresh answers a new one. Using a refresh token MUST be
atomic in the storage: of concurrent uses exactly one consumes it. Every other
use within a short grace (two tabs renewing together, a lost response) MUST
answer the same successor, so a sign-in's chain never forks; a use after the
grace MUST revoke the whole sign-in and be refused. A revoked or expired
sign-in MUST NOT be renewed. Other clients' refresh tokens are not rotated.

r[auth.oidc.revocation]
The revocation endpoint (`/oauth2/revoke`, RFC 7009) MUST end the sign-in
(grant) of the refresh or access token it is given, when the token was issued
to the calling client, by one revocation mark that every use of the sign-in's
tokens checks, however many it minted: no refresh token of the sign-in is
accepted, and its access tokens are refused where the issuer takes them in
place of a session, at `/oauth2/userinfo` and at `/oauth2/linked-token`.
The issuer session the person signed in with MUST stay. An unknown or already
revoked token MUST NOT be an error, and revoking a sign-in already revoked MUST
NOT store anything new.

r[auth.oidc.userinfo]
The userinfo endpoint MUST validate the access token and return claims
according to granted scopes.

r[auth.oidc.jwks]
The OIDC provider MUST publish JWKS metadata through the shared JWT key set
surface.

r[auth.oidc.plugin-descriptor]
OIDC provider behavior MUST be represented by an Architect auth plugin
descriptor with upstream Better Auth parity, dependencies, and capability
metadata.

r[auth.oidc.plugin-routes]
The OIDC provider plugin descriptor MUST reference generated command
metadata so Rust, Axum, Vox, and OpenAPI expose the same operation ids.
