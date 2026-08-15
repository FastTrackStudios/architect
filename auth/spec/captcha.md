+++
title = "CAPTCHA"
description = "Rules for CAPTCHA verification hooks."
weight = 98
+++

# CAPTCHA

r[auth.captcha.verify]
CAPTCHA verification MUST expose a typed command for validating a token for
a configured auth flow.

r[auth.captcha.providers]
CAPTCHA providers MUST support disabled, bypass, deterministic test, and
fail-closed modes.

r[auth.captcha.signup-hook]
When sign-up is configured as protected, sign-up MUST reject missing or
failed CAPTCHA checks before creating user, credential, or session state.

r[auth.captcha.errors]
Failed provider checks MUST return permission-denied errors and malformed
CAPTCHA metadata MUST return invalid-input errors.

r[auth.captcha.plugin-descriptor]
The CAPTCHA plugin MUST declare its Better Auth lineage, provider
capabilities, test/bypass/fail-closed modes, protected-flow hooks, and stable
error behavior.

r[auth.captcha.plugin-routes]
The CAPTCHA plugin routes MUST be generated from the shared auth command
catalog so Rust, Axum, Vox, and OpenAPI expose the same operation ids and
session requirements.
