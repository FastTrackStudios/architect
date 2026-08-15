+++
title = "Admin and user management"
description = "Rules for privileged user and session management."
weight = 110
+++

# Admin and User Management

r[auth.admin.requires-role]
Admin commands MUST require a server-verified admin role or permission.

r[auth.admin.list-users]
User listing MUST support pagination and MUST NOT expose password
hashes, raw tokens, backup codes, or encrypted secret payloads.

r[auth.admin.create-user]
Admin user creation MUST create a verified user, optionally attach a
credential password account, and reject duplicate email addresses.

r[auth.admin.set-user-password]
Admin password setting MUST create or update the target user's credential
account without storing the raw password.

r[auth.admin.ban]
Banning a user MUST prevent new sign-ins while the ban is active.

r[auth.admin.ban-expiry]
A ban with an expiry MUST stop applying after `ban_expires`.

r[auth.admin.revoke-sessions]
Admin session revocation MUST deactivate the targeted sessions.

r[auth.admin.list-user-sessions]
Admin user-session listing MUST return sessions for the targeted user
without exposing raw session tokens.

r[auth.admin.revoke-session]
Admin single-session revocation MUST deactivate only a session that
belongs to the targeted user.

r[auth.admin.impersonate]
Admin impersonation MUST create a distinguishable session with
`impersonated_by` set to the admin user's ID.

r[auth.admin.stop-impersonating]
Stopping impersonation MUST deactivate the current impersonated session.

r[auth.admin.remove-user]
Admin user removal MUST delete the targeted user and revoke or remove
their active sessions and credentials.

r[auth.admin.has-permission]
Admin permission checks MUST expose the same resource/action decision
model used by organization authorization.

r[auth.admin.audit]
Privileged admin commands SHOULD emit audit events containing actor,
target, action, and timestamp.

r[auth.admin.no-self-lockout]
Admin role changes SHOULD prevent the last admin from removing their own
admin access unless another recovery path is configured.

r[auth.admin.plugin-descriptor]
Admin behavior MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.admin.plugin-routes]
The admin plugin descriptor MUST reference generated command metadata
for privileged user and session management transport/OpenAPI exposure.
