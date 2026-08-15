---
title = "OAuth proxy"
description = "Rules for proxying OAuth callbacks across deployment origins."
---

# OAuth Proxy

r[auth.oauth-proxy.metadata]
OAuth proxy metadata MUST expose current URL, production URL, callback URL,
proxy decision, and available OAuth provider metadata.

r[auth.oauth-proxy.state]
Proxy authorization MUST create normal OAuth state so callback forwarding
composes with generic OAuth and built-in OAuth providers.

r[auth.oauth-proxy.callback-forwarding]
Callback forwarding MUST package provider id, state, callback URL, profile
data, and timestamp into an encrypted payload that another trusted
environment can consume.

r[auth.oauth-proxy.redirect-policy]
Proxy authorization, forwarding, and consumption MUST reject callback URLs
outside configured trusted redirect origins.

r[auth.oauth-proxy.max-age]
Encrypted callback payloads MUST expire after a short configured max age and
MUST reject implausible future timestamps.

r[auth.oauth-proxy.provider-composition]
The OAuth proxy plugin MUST expose provider metadata from the same provider
registry used by the OAuth plugin.

r[auth.oauth-proxy.plugin-descriptor]
OAuth proxy behavior MUST be represented by an Architect auth plugin
descriptor with upstream Better Auth parity, dependencies, and capability
metadata.

r[auth.oauth-proxy.plugin-routes]
The OAuth proxy plugin descriptor MUST reference generated command metadata
so Rust, Axum, Vox, and OpenAPI expose the same operation ids.
