+++
title = "Password management"
description = "Rules for changing and resetting passwords."
weight = 40
+++

# Password Management

r[auth.password.change.requires-current]
Changing a password from an authenticated session MUST require the
current password unless the command is executed through an authorized
admin flow.

r[auth.password.change-invalidates]
Successful password change SHOULD invalidate other active sessions for
the same user unless the caller explicitly requests otherwise and policy
allows it.

r[auth.password.reset-token-random]
Password reset tokens MUST be generated from cryptographically secure
randomness.

r[auth.password.reset-token-hash]
Only a hash of the password reset token MUST be stored.

r[auth.password.reset-expiry]
Password reset tokens MUST expire and expired tokens MUST fail without
changing credentials.

r[auth.password.reset-single-use]
Password reset tokens MUST be single-use.

r[auth.password.reset-generic-response]
Requesting a password reset for an unknown email MUST return the same
public response shape as requesting one for a known email.

r[auth.password.strength-policy]
Password creation, change, and reset MUST apply the configured password
strength policy before hashing.

r[auth.password.plugin-descriptor]
Password management MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.password.plugin-routes]
The password management plugin descriptor MUST reference generated
command metadata for change-password, request-password-reset, and
complete-password-reset transport/OpenAPI exposure.
