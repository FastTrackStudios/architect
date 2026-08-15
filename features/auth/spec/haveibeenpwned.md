+++
title = "HaveIBeenPwned"
description = "Rules for breached-password checks."
weight = 78
+++

# HaveIBeenPwned

r[auth.hibp.range-check]
The breached-password plugin MUST use a k-anonymity password hash shape
with a five-character SHA-1 prefix and suffix matching.

r[auth.hibp.reject-breached]
Configured breached passwords MUST be rejected before signup, password
change, password reset, anonymous upgrade, and admin password creation or
reset persist a password hash.

r[auth.hibp.failure-policy]
Provider failures MUST follow the configured failure policy: fail-open
allows the password and fail-closed denies the operation.

r[auth.hibp.password-hooks]
Breached-password checks MUST run after password strength validation and
before hashing or persistence.

r[auth.hibp.plugin-descriptor]
The breached-password capability MUST be represented by an Architect auth
plugin descriptor with a stable id, upstream parity target,
dependencies, and capability names.

r[auth.hibp.plugin-routes]
The breached-password plugin descriptor MUST own password-check route
metadata used by transport and OpenAPI adapters.

The current implementation provides a deterministic test provider and an
unavailable-provider mode for CI. A production HTTP range-query client is
the remaining compatibility boundary for deployments that need live HIBP
queries.
