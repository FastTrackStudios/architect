+++
title = "OAuth and account linking"
description = "Rules for provider accounts, OAuth tokens, and linking."
weight = 60
+++

# OAuth and Account Linking

r[auth.oauth.provider-account-unique]
The pair `(provider_id, account_id)` MUST identify at most one linked
account.

r[auth.oauth.link-authenticated]
Linking a provider account to an existing user MUST require an
authenticated session for that user or an explicit trusted callback flow.

r[auth.oauth.signin-existing-account]
OAuth sign-in for an already linked provider account MUST sign in the
owning user.

r[auth.oauth.signin-new-account]
OAuth sign-in for an unlinked provider account MAY create a new user
only when the provider and runtime configuration allow registration.

r[auth.oauth.email-trust]
Provider email claims MUST NOT mark a user email verified unless the
provider reports verified email status or is configured as trusted.

r[auth.oauth.token-encryption]
Access tokens, refresh tokens, and ID tokens MUST be encrypted before
storage.

r[auth.oauth.unlink-last-credential]
Unlinking a provider account MUST be rejected when it would leave the
user without any usable sign-in credential, unless an admin recovery
policy allows it.

r[auth.oauth.state-csrf]
OAuth authorization flows MUST validate a server-generated state value
to prevent CSRF.

r[auth.oauth.access-token]
Access-token retrieval MUST be scoped to the linked account owner and
MUST decrypt stored token envelopes only at the typed command boundary.

r[auth.oauth.refresh-token]
Refresh-token behavior MUST update encrypted stored OAuth tokens without
requiring raw OAuth secrets to be stored in account records.

r[auth.oauth.provider-registry]
The OAuth provider registry MUST include Google, GitHub, and Discord
provider descriptors.

r[auth.oauth.generic-provider]
Generic OAuth providers MUST be representable by the same provider
descriptor shape as built-in providers.

r[auth.oauth.plugin-descriptor]
OAuth and generic OAuth behavior MUST be represented by an Architect auth
plugin descriptor with a stable id, upstream parity target,
dependencies, and capability names.

r[auth.oauth.plugin-routes]
The OAuth plugin descriptor MUST reference generated command metadata
for social sign-in, callback state verification, and account linking
transport/OpenAPI exposure.
