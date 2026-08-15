+++
title = "Multi-session"
description = "Rules for maintaining and switching between multiple client-held sessions."
weight = 73
+++

# Multi-session

r[auth.multisession.list]
A client MAY present a device-local set of session tokens and receive the
valid active sessions for that device without failing the whole request
for unknown, forged, expired, or revoked tokens.

r[auth.multisession.no-forged-sessions]
Unknown, forged, expired, or inactive session tokens MUST NOT produce a
device session entry.

r[auth.multisession.set-active]
A client MUST be able to select one valid token from its device-local
session token set as the active session and receive that session's own
user, session, and raw token.

r[auth.multisession.permission-isolation]
Switching the active session MUST NOT transfer roles, organization
context, permissions, or admin privileges from one user session to
another.

r[auth.multisession.revoke]
A client MUST be able to revoke a session token that belongs to its
device-local token set.

r[auth.multisession.current-session]
When the revoked session is the active session, the result SHOULD include
the next valid device session when one exists; the revoked token MUST NOT
continue to authenticate.

r[auth.multisession.plugin-descriptor]
The multi-session capability MUST be represented by an Architect auth
plugin descriptor with a stable id, upstream parity target,
dependencies, and capability names.

r[auth.multisession.plugin-routes]
The multi-session plugin descriptor MUST own list-device-sessions,
set-active, and revoke route metadata used by transport and OpenAPI
adapters.
