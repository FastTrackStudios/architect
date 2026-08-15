+++
title = "Storage and migrations"
description = "Rules for repositories, backends, and database migrations."
weight = 130
+++

# Storage and Migrations

r[auth.storage.repo-first]
Storage backends MUST implement Architect-style repo or service
contracts rather than exposing SQL assembly through public adapters.

r[auth.storage.backend-parity]
All supported backends MUST preserve the same observable auth behavior
for the same command inputs.

r[auth.storage.migrations-create-tables]
The SeaORM migrator MUST create tables for every `auth-proto` entity
required by enabled storage features.

r[auth.storage.migration-compatibility]
Migration tests MUST prove the current supported deployed schema can be
migrated repeatedly without dropping auth tables or indexes.

r[auth.storage.database-matrix]
The supported database matrix MUST document which databases are CI-backed
today and which databases are planned but not yet supported.

r[auth.storage.plugin-migrations]
Every auth plugin MUST declare its storage tables, indexes, and migration
ids, including an explicit empty declaration for stateless plugins.

r[auth.storage.unique-indexes]
The storage layer MUST enforce uniqueness for user email, username,
provider account identity, organization slug, member identity, API key
hash, and passkey credential ID where those fields are enabled.

r[auth.storage.lookup-indexes]
The storage layer SHOULD index token hashes, verification identifiers,
user foreign keys, organization foreign keys, and expiry fields used in
normal auth queries.

r[auth.storage.transactions]
Multi-record auth changes, such as signup plus account creation plus
session creation, MUST be atomic when the backend supports
transactions.

r[auth.storage.clock]
Storage-generated timestamps MUST follow the same server clock semantics
as runtime-generated timestamps.

r[auth.storage.no-public-sql]
Public APIs MUST NOT require callers to pass raw SQL strings.
