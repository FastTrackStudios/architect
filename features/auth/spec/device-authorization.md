+++
title = "Device authorization"
description = "Rules for OAuth 2.0 device authorization grants."
weight = 95
+++

# Device Authorization

r[auth.device.create]
Device authorization MUST issue an opaque device code, user code,
verification URI, expiry, and polling interval.

r[auth.device.verify]
Verification UI backends MUST be able to verify an unexpired user code and
return the requesting client metadata.

r[auth.device.approve-deny]
User-code approval MUST require an authenticated session, and denial MUST
cause device polling to fail without issuing a session.

r[auth.device.polling]
Polling before approval MUST return authorization-pending semantics, and
polling faster than the issued interval MUST return slow-down semantics.

r[auth.device.expiry]
Expired device codes and user codes MUST fail.

r[auth.device.plugin-descriptor]
The device authorization plugin MUST declare its Better Auth lineage,
verification-token dependency, and capabilities for device codes, user
codes, verification UI, polling, slow-down, approval, denial, and expiry.

r[auth.device.plugin-routes]
The device authorization plugin routes MUST be generated from the shared
auth command catalog so Rust, Axum, Vox, and OpenAPI expose the same
operation ids and session requirements.
