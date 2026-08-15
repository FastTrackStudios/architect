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
