+++
title = "Username"
description = "Rules for username-based account identifiers and sign-in."
weight = 75
+++

# Username

r[auth.username.validation]
Usernames MUST be trimmed, 3 to 32 characters long, and contain only
ASCII letters, ASCII numbers, and underscore.

r[auth.username.reserved]
Reserved names such as `admin`, `api`, `auth`, `root`, `security`,
`support`, `system`, and `www` MUST be rejected.

r[auth.username.case-insensitive]
Username storage and lookup MUST be case-insensitive. The canonical
stored username is lowercase; display username may preserve caller
capitalization.

r[auth.username.unique]
Canonical usernames MUST be unique across users for signup and update
flows.

r[auth.username.signin]
Username/password sign-in MUST authenticate against the canonical
username and password account, apply the same banned and email
verification checks as email/password sign-in, and issue a session on
success.

r[auth.username.update]
An authenticated user MUST be able to update their username and display
username when the new canonical username is valid, unreserved, and
unique.

r[auth.username.plugin-descriptor]
The username capability MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.username.plugin-routes]
The username plugin descriptor MUST own username sign-in and username
update route metadata used by transport and OpenAPI adapters.
