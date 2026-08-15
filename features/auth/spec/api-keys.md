+++
title = "API keys"
description = "Rules for API key issuance, lookup, permissions, and limits."
weight = 100
+++

# API Keys

r[auth.apikey.random]
API keys MUST be generated from cryptographically secure randomness.

r[auth.apikey.hash-storage]
Only a hash of the API key secret MUST be stored.

r[auth.apikey.prefix]
API keys SHOULD include a non-secret prefix for display and lookup.

r[auth.apikey.raw-return-once]
The raw API key MUST only be returned when it is created.

r[auth.apikey.list]
API key listing MUST be scoped to the authenticated owner.

r[auth.apikey.get]
API key lookup MUST return only keys owned by the authenticated user.

r[auth.apikey.update]
API key updates MUST be owner-scoped and preserve hash-only secret
storage.

r[auth.apikey.delete]
API key deletion MUST remove the key for future lookup and
authentication.

r[auth.apikey.verify]
API key verification MUST authenticate the key and optionally enforce
the requested permission.

r[auth.apikey.disabled]
Disabled API keys MUST fail authentication.

r[auth.apikey.expired]
Expired API keys MUST fail authentication.

r[auth.apikey.permissions]
API key authorization MUST deny by default when permissions do not grant
the requested action.

r[auth.apikey.rate-limit]
When rate limiting is enabled for an API key, requests beyond the
configured window limit MUST fail without executing the protected
command.

r[auth.apikey.revoke]
Revoking an API key MUST make it unusable for future authentication.

r[auth.apikey.plugin-descriptor]
API key behavior MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.apikey.plugin-routes]
The API key plugin descriptor MUST reference generated command metadata
for API key transport/OpenAPI exposure.
