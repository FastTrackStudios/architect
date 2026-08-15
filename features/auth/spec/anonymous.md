+++
title = "Anonymous users"
description = "Rules for anonymous users and account upgrade."
weight = 96
+++

# Anonymous Users

r[auth.anonymous.signin]
Anonymous sign-in MUST create a user and active session without a permanent
credential.

r[auth.anonymous.policy]
Anonymous sessions MUST carry an anonymous role and MUST NOT bypass normal
authorization policy checks.

r[auth.anonymous.link]
Anonymous users MUST be upgradable into a permanent email/password account
without changing the user id.

r[auth.anonymous.revoke-obsolete]
Upgrading an anonymous account MUST revoke obsolete anonymous sessions and
issue a fresh permanent session.

r[auth.anonymous.cleanup]
Anonymous cleanup MUST require an admin session and delete only stale
anonymous users.

r[auth.anonymous.plugin-descriptor]
The anonymous plugin MUST declare its Better Auth lineage, session and
email-password dependencies, and capabilities for anonymous sessions,
account upgrade, policy roles, cleanup, and session revocation.

r[auth.anonymous.plugin-routes]
The anonymous plugin routes MUST be generated from the shared auth command
catalog so Rust, Axum, Vox, and OpenAPI expose the same operation ids and
session requirements.
