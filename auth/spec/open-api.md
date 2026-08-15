+++
title = "OpenAPI"
description = "Rules for plugin-aware OpenAPI generation."
weight = 82
+++

# OpenAPI

r[auth.openapi.plugin-descriptor]
The OpenAPI capability MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.openapi.plugin-routes]
The OpenAPI plugin descriptor MUST own the OpenAPI document route
metadata used by transport adapters.

r[auth.openapi.plugin-aware]
Generated OpenAPI output MUST include plugin ownership metadata for every
operation and document-level metadata for enabled auth plugins.

r[auth.openapi.request-schemas]
Generated OpenAPI output MUST include request body schemas for every
command with a JSON request body.

r[auth.openapi.response-schemas]
Generated OpenAPI output MUST include response schemas for every command
response type.

r[auth.openapi.error-schemas]
Generated OpenAPI output MUST include stable public error schemas for
non-success responses.

r[auth.openapi.snapshot]
Generated OpenAPI output MUST be covered by a stable snapshot-style test
that verifies paths, components, plugin metadata, and security metadata.
