+++
title = "Magic link"
description = "Rules for magic-link sign-in."
weight = 100
+++

# Magic Link

r[auth.magic.send]
Magic-link send MUST create a link with a random token for the canonical
email address and return it through the Rust API for deterministic email-sink
tests.

r[auth.magic.verify]
Magic-link verification MUST validate the submitted token against the stored
hash and canonical email address.

r[auth.magic.expiry]
Expired magic-link tokens MUST fail verification.

r[auth.magic.single-use]
Magic-link tokens MUST be single-use.

r[auth.magic.session]
Successful magic-link verification MUST issue a session.

r[auth.magic.redirect-trust]
Magic-link send and verify MUST reject redirect URLs outside the configured
auth base URL.

r[auth.magic.plugin-descriptor]
The magic-link plugin MUST declare its Better Auth lineage,
verification-token dependency, and capabilities for link generation, test
sink, token hash storage, expiration, single-use verification, session
creation, and redirect trust.

r[auth.magic.plugin-routes]
The magic-link plugin routes MUST be generated from the shared auth command
catalog so Rust, Axum, Vox, and OpenAPI expose the same operation ids and
session requirements.
