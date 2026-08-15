+++
title = "Error taxonomy"
description = "Rules for stable public auth errors across protocols."
weight = 83
+++

# Error taxonomy

r[auth.errors.taxonomy]
Every public `AuthFlowError` variant MUST have a stable taxonomy entry
with a Rust variant name, public code, public message, HTTP status, and
Vox status.

r[auth.errors.axum]
Axum responses MUST derive status, code, and message from the stable auth
error taxonomy.

r[auth.errors.vox]
Vox integration MUST derive protocol status categories from the stable
auth error taxonomy.

r[auth.errors.openapi]
OpenAPI output MUST document public auth error responses with the same
stable error codes used by Rust, Axum, and Vox mappings.

r[auth.errors.coverage]
Tests MUST cover representative mappings for credentials, validation,
permission, verification, two-factor, session-expiry, and internal
failures.
