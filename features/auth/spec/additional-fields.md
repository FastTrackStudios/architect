+++
title = "Additional fields"
description = "Rules for Better Auth additional fields and hidden metadata parity."
weight = 81
+++

# Additional fields

r[auth.additional-fields.persist]
Additional fields MUST be validated against typed field specs before
service-defined metadata is persisted.

r[auth.additional-fields.returned]
Additional field projection MUST return only fields whose spec has
`returned = true`.

r[auth.additional-fields.hidden-metadata]
Fields whose spec has `returned = false` MUST remain persisted but hidden
from public API response projections.

r[auth.additional-fields.schema]
Protocol and OpenAPI metadata MUST expose the additional-field schema so
clients can distinguish exposed fields from hidden metadata.

r[auth.additional-fields.migration]
Until Architect emits physical columns for service-defined fields,
additional user fields MAY be stored in `metadata_json`; session and
account field specs MUST still be declared so migrations can add physical
columns later without changing the public schema contract.

r[auth.additional-fields.plugin-descriptor]
The additional-fields capability MUST be represented by an Architect auth
plugin descriptor with a stable id, upstream parity target,
dependencies, and capability names.

r[auth.additional-fields.plugin-routes]
The additional-fields plugin descriptor MUST own schema route metadata
used by transport and OpenAPI adapters.
