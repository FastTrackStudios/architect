+++
title = "Errors"
description = "RepoError's four variants and when each is right, ClientError<E> on the client, and the blessed pattern for a custom service error enum."
weight = 57
+++

architect keeps two error layers deliberately separate: **what the
service failed with** (a typed app error that travels the wire) and
**what the infrastructure failed with** (connect/transport problems the
client wraps around any call). A page can `match` on
`App(RepoError::NotFound)` while every connectivity problem gets one
shared "reconnecting…" treatment.

## `RepoError` — the repo wire error

Every architect-generated repo method returns
`Result<_, architect::RepoError>`. The enum is **tight on purpose** —
new variants are a wire-format change for every existing `<Entity>Repo`
trait:

```rust
#[derive(Debug, Clone, PartialEq, Eq, facet::Facet, thiserror::Error)]
#[repr(u8)]
pub enum RepoError {
    #[error("entity not found")]
    NotFound,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("internal error: {0}")]
    Internal(String),
}
```

When each is right:

| Variant | Use it when | Examples |
|---------|-------------|----------|
| `NotFound` | The id resolved to nothing. The client's cache-first hooks render this as the typed "doesn't exist" state. | `get`/`update`/`delete` on a missing row |
| `InvalidInput(msg)` | The request itself is malformed — the caller can fix it and retry. | unknown sort field, unparseable id, validation failure |
| `Conflict(msg)` | The request was well-formed but lost to current state. | duplicate caller-supplied id (a String-pk slug that already exists), optimistic-lock failure |
| `Internal(msg)` | The backend broke; nothing the caller did. Storage layers fold their native errors here (`map_err(\|e\| RepoError::Internal(e.to_string()))`). | DB connection lost, codec failure |

## `ClientError<E>` — the client envelope

On the client, a data hook can fail in more ways than the handler can:
the connection may never come up, or the call may die in flight. So the
hooks (and everything the derive emits) fail with
`architect::ClientError<E>`, where `E` is the service's typed error:

- **`App(E)`** — the handler ran and returned the service's typed error
  (vox's `VoxError::User`). This is the arm feature pages branch on:
  `App(RepoError::NotFound)` renders the "not found" state.
- **`Connect(String)`** — establishing the connection failed (after the
  transport's retry policy gave up).
- **`Transport { detail, retryable }`** — the call failed in flight
  (connection closed, payload mismatch); `retryable` is vox's verdict
  on whether a fresh connection could succeed.

`is_infrastructure()` is the "show the reconnecting banner" test;
`app()` extracts the typed error. The derive emits a per-entity alias —
`type ExampleClientError = ClientError<RepoError>;` — and a
`From<vox::VoxError<E>>` impl folds vox call errors into the envelope,
so `?`/`map_err(ClientError::from)` is all a loader needs.

## Custom service errors — the blessed pattern

CRUD rides `RepoError`, but a hand-written `#[vox::service]` usually has
domain failures `RepoError` shouldn't absorb (remember: extending
`RepoError` changes the wire for *every* repo). The pattern: **one
dedicated error enum per service**, `facet::Facet` so it travels the
vox wire, `thiserror::Error` for ergonomic `?` on the server, kept
separate from `RepoError`. From
`examples/app/features/example/example-proto/src/error.rs`:

```rust
//! Error type for the custom [`ExampleService`](crate::ExampleService).
//!
//! A `facet::Facet` enum so it travels over the vox wire, and
//! `thiserror::Error` for ergonomic `?` on the server. `#[repr(u8)]` keeps
//! the wire tag compact. Add variants here as the service grows.

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet, thiserror::Error)]
#[repr(u8)]
pub enum ExampleServiceError {
    #[error("example not found")]
    NotFound,
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("internal error: {0}")]
    Internal(String),
}
```

and the service methods return it directly:

```rust
#[vox::service]
pub trait ExampleService {
    async fn search(&self, query: String, limit: u32)
        -> Result<Vec<Example>, ExampleServiceError>;
}
```

The checklist for your own service error:

1. **`facet::Facet` + `#[repr(u8)]`** — it's a wire type; the compact
   tag matters.
2. **`thiserror::Error`** — `Display` drives both server logs and the
   client's `Notifications` toast on rollback.
3. **`Clone + PartialEq`** — client hooks store and compare errors
   across renders.
4. **Don't reuse or extend `RepoError`.** A service that touches a repo
   internally maps at the boundary
   (`repo.get(id).await.map_err(|e| match e { RepoError::NotFound =>
   ExampleServiceError::NotFound, other =>
   ExampleServiceError::Internal(other.to_string()) })?`) so the repo's
   wire contract and the service's stay independently evolvable.

On the client the same envelope applies — the hook fails with
`ClientError<ExampleServiceError>`, and pages branch on
`App(ExampleServiceError::NotFound)` exactly as they do for repos.
