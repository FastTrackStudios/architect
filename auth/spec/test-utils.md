+++
title = "Test utilities"
description = "Rules for downstream auth integration test helpers."
weight = 150
+++

# Test Utilities

r[auth.test-utils.fixtures]
Test utilities MUST expose a reusable auth harness for integration tests,
including an in-memory SQLite fixture when the database feature is
enabled.

r[auth.test-utils.users]
Test utilities MUST create users through typed auth commands rather than
by hand-building storage records.

r[auth.test-utils.sessions]
Test utilities MUST create and sign in sessions through the runtime API.

r[auth.test-utils.organizations]
Test utilities MUST provide helpers for organizations, roles, and
invitations.

r[auth.test-utils.teams]
Test utilities MUST provide helpers for organization teams.

r[auth.test-utils.api-keys]
Test utilities MUST provide helpers for API key fixtures.

r[auth.test-utils.otp]
Test utilities MUST expose deterministic OTP send/verify helpers for
tests.

r[auth.test-utils.cookies]
Test utilities MUST expose cookie and bearer header builders.

r[auth.test-utils.axum]
Test utilities SHOULD expose Axum header helpers when the Axum feature is
enabled.

r[auth.test-utils.vox]
Test utilities SHOULD expose Vox bearer middleware helpers when the Vox
feature is enabled.
