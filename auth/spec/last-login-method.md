+++
title = "Last login method"
description = "Rules for tracking the most recent successful authentication method."
weight = 74
+++

# Last Login Method

r[auth.lastlogin.track-email]
Successful email/password sign-up and sign-in flows MUST record `email`
as the user's last login method.

r[auth.lastlogin.track-oauth]
Successful OAuth sign-in flows MUST record the provider id as the user's
last login method.

r[auth.lastlogin.track-passkey]
Successful passkey authentication flows MUST record `passkey` as the
user's last login method.

r[auth.lastlogin.track-email-otp]
Successful email OTP verification that creates a session MUST record
`email-otp` as the user's last login method.

r[auth.lastlogin.track-magic-link]
Successful magic-link verification MUST record `magic-link` as the
user's last login method.

r[auth.lastlogin.track-anonymous-upgrade]
Anonymous sign-in MUST record `anonymous`, and anonymous email/password
upgrade MUST record `email`.

r[auth.lastlogin.query]
An authenticated caller MUST be able to query the persisted last login
method for the current user.

r[auth.lastlogin.clear]
An authenticated caller MUST be able to clear the persisted last login
method for the current user.

r[auth.lastlogin.cookie-config]
The plugin MUST expose Better Auth-compatible client cookie metadata:
the default cookie name `better-auth.last_used_login_method`, a 30-day
max age, and a client-readable cookie policy. Custom cookie names,
custom database column names, and custom method resolver callbacks are
documented compatibility concerns until Architect gains plugin-specific
runtime configuration.

r[auth.lastlogin.plugin-descriptor]
The last-login-method capability MUST be represented by an Architect
auth plugin descriptor with a stable id, upstream parity target,
dependencies, and capability names.

r[auth.lastlogin.plugin-routes]
The last-login-method plugin descriptor MUST own query and clear route
metadata used by transport and OpenAPI adapters.
