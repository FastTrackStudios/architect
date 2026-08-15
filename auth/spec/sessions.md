+++
title = "Sessions"
description = "Rules for session creation, lookup, renewal, and sign-out."
weight = 30
+++

# Sessions

r[auth.sessions.token-random]
Session tokens MUST be generated from cryptographically secure
randomness with enough entropy to resist online guessing.

r[auth.sessions.token-hash-storage]
Only a hash of the session token MUST be stored. The raw token MUST only
be returned at session creation time.

r[auth.sessions.ttl]
New sessions MUST expire according to `session_ttl_seconds` unless a
more specific configured policy overrides it.

r[auth.sessions.current.valid]
`current_session` with a valid, active, unexpired token MUST return the
matching user and session.

r[auth.sessions.current.missing]
`current_session` with an unknown token MUST return invalid credentials
or session expired without revealing whether a similar token exists.

r[auth.sessions.current.expired]
`current_session` with an expired token MUST return `SessionExpired` and
MUST NOT silently extend the session.

r[auth.sessions.signout]
`sign_out` MUST revoke or deactivate the matching session so the same
token cannot be used again.

r[auth.sessions.signout-idempotence]
`sign_out` for an unknown, expired, or already inactive token MUST NOT
reveal token existence.

r[auth.sessions.refresh]
`refresh_session` with a valid, active, unexpired token MUST issue a
fresh session for the same user with a new expiry, preserving the
original session's context metadata (IP address, user agent,
impersonation, active organization).

r[auth.sessions.refresh-rotation]
`refresh_session` MUST rotate the token: the replacement session gets a
new token, and the refreshed session MUST be deactivated so its token
cannot be used again.

r[auth.sessions.refresh-invalid]
`refresh_session` with an unknown, expired, or inactive token MUST fail
with the same errors as `current_session` and MUST NOT issue a session.

r[auth.sessions.context]
Session creation SHOULD persist optional IP address and user-agent
metadata when provided by the caller.

r[auth.sessions.impersonation]
Impersonated sessions MUST record `impersonated_by` and remain
distinguishable from normal user sessions in admin-visible APIs.

r[auth.sessions.list]
An authenticated caller MUST be able to list their own sessions without
exposing sessions that belong to other users.

r[auth.sessions.revoke]
An authenticated caller MUST be able to revoke one of their own sessions
by session id and MUST NOT revoke another user's session through this
command.

r[auth.sessions.revoke-other]
An authenticated caller MUST be able to revoke all of their other
sessions while preserving the current session.

r[auth.sessions.plugin-descriptor]
The session management capability MUST be represented by an Architect
auth plugin descriptor with a stable id, upstream parity target,
dependencies, and capability names.

r[auth.sessions.plugin-routes]
The session management plugin descriptor MUST own current-session,
sign-out, list-sessions, revoke-session, and revoke-other-sessions route
metadata used by transport and OpenAPI adapters.
