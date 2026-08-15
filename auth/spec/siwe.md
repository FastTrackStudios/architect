+++
title = "SIWE"
description = "Rules for Sign-In with Ethereum nonce verification and account linking."
weight = 77
+++

# SIWE

r[auth.siwe.nonce]
The SIWE plugin MUST issue short-lived single-use nonces.

r[auth.siwe.verify]
SIWE message verification MUST parse the domain, address, and nonce,
verify the signature through the configured verifier, and issue a
session on success.

r[auth.siwe.domain]
SIWE messages MUST be rejected when the message domain does not match
the configured auth domain.

r[auth.siwe.replay]
SIWE nonces MUST be consumed on successful verification and MUST NOT be
accepted again.

r[auth.siwe.linked-account]
If a SIWE address is already linked to an account, verification MUST
sign in that linked user instead of creating a duplicate user.

r[auth.siwe.signup]
If a SIWE address is not linked and signup is enabled, verification MUST
create a user and link the SIWE address account before issuing a session.

r[auth.siwe.address-link]
An authenticated user MUST be able to link a valid SIWE address when it
is not already linked to another account.

r[auth.siwe.plugin-descriptor]
The SIWE capability MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.siwe.plugin-routes]
The SIWE plugin descriptor MUST own nonce, verify, and link route
metadata used by transport and OpenAPI adapters.

The current verifier is a deterministic test verifier so the crate can
exercise nonce, parser, domain, linking, and replay semantics without
adding Ethereum recovery dependencies. Production EIP-191/secp256k1
recovery remains a documented compatibility boundary.
