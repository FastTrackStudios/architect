+++
title = "Organizations and RBAC"
description = "Rules for organizations, members, invitations, and roles."
weight = 70
+++

# Organizations and RBAC

r[auth.org.slug-unique]
Organization slugs MUST be unique.

r[auth.org.create-owner]
Creating an organization MUST create an owner membership for the
creating user.

r[auth.org.member-unique]
A user MUST have at most one active membership per organization.

r[auth.org.role-authoritative]
Authorization checks MUST use server-stored membership roles, not
client-submitted role claims.

r[auth.org.invite-token]
Invitations MUST use a non-guessable acceptance path or token and MUST
not allow acceptance after expiry.

r[auth.org.invite-status]
Invitation acceptance, rejection, and cancellation MUST update
`InvitationStatus` exactly once from `Pending`.

r[auth.org.remove-last-owner]
Removing or demoting the last owner of an organization MUST be rejected
unless the organization is being deleted.

r[auth.org.active-session]
When a session has an active organization, commands scoped to that
organization MUST confirm the user is still an active member.

r[auth.org.rbac-deny-default]
Organization authorization MUST deny by default when no rule grants the
requested action.

r[auth.org.permission-resources]
Organization authorization MUST model permissions as typed resource and
action pairs. The default resource actions are `organization:update`,
`organization:delete`, `member:create`, `member:update`, `member:delete`,
`invitation:create`, `invitation:cancel`, `team:create`, `team:update`,
`team:delete`, `ac:create`, `ac:read`, `ac:update`, and `ac:delete`.

r[auth.org.default-permission-roles]
Default organization roles MUST match better-auth semantics: `owner`
can manage organization, members, invitations, teams, and access control;
`admin` can manage members, invitations, teams, and access control and
can update but not delete organizations; `member` is denied management
actions and can read access-control metadata.

r[auth.org.composite-roles]
Authorization MUST grant access when any role in a comma-separated
membership role list grants the requested permission.

r[auth.org.dynamic-access-control]
Organizations MAY define per-organization role permissions. Dynamic
role permissions MUST be stored server-side and merged with default role
permissions during authorization.

r[auth.org.teams]
When teams are enabled, teams MUST belong to an organization and team
membership MUST be unique for `(team_id, user_id)`.

r[auth.org.plugin-descriptor]
Organization, team, RBAC, and access-control behavior MUST be
represented by an Architect auth plugin descriptor with a stable id,
upstream parity target, dependencies, and capability names.

r[auth.org.plugin-routes]
The organization plugin descriptor MUST reference generated command
metadata for organization, membership, invitation, dynamic role,
permission, and team transport/OpenAPI exposure.
