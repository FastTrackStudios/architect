+++
title = "MCP"
description = "Rules for MCP authorization bridge support."
weight = 79
+++

# MCP

r[auth.mcp.session]
MCP authorization requests MUST validate architect-auth session tokens or
bearer authorization headers before returning a service authorization
result.

r[auth.mcp.permissions]
When an MCP authorization request includes organization, resource, and
action fields, it MUST apply the same organization permission checks as
normal service commands.

r[auth.mcp.api-key]
When an MCP authorization request is backed by an API key bearer token,
resource and action fields MUST be checked against the API key permission
set before returning an allowed service authorization result.

r[auth.mcp.plugin-descriptor]
The MCP capability MUST be represented by an Architect auth plugin
descriptor with a stable id, upstream parity target, dependencies, and
capability names.

r[auth.mcp.plugin-routes]
The MCP plugin descriptor MUST own MCP authorization route metadata used
by transport and OpenAPI adapters.
