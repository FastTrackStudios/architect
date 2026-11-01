+++
title = "Errors"
description = "One typed error per service, the same on every wire: #[architect::error], ClientError, the HTTP envelope."
weight = 13
+++

## What it is and when to use it

architect keeps two kinds of failure apart: the service's own typed error,
which travels the wire as a value, and infrastructure failures (no
connection, a dropped link, a bad body). `#[architect::error]` on an enum
writes its wire derives, an `HttpError` impl (status and code per variant), a
`Transport(String)` variant and a `From<architect::TransportError>`. Clients
fail with `ClientError<E>` (`App`, `Connect`, `Transport`), and
`vox_into_app` / `ClientError::into_app` fold infrastructure failures back
into `E`, so a caller matches one enum.

Write one error enum per service. On HTTP an error is
`{code, message, error}` with the variant's status. Design:
[Errors](@/architecture/errors.md).

## A minimal example

From `examples/greeter/src/lib.rs`. The crate needs `thiserror` as a dependency.

```rust
#[architect::error]
#[derive(Eq)]
pub enum GreetError {
    #[error("nobody is called {0}")]
    #[architect(http_status = 404)]     // also: code = "...", transport
    Unknown(String),
}
```

That adds `GreetError::Transport(String)` (503, code `transport`). Over HTTP
a miss answers:

```text
404 {"code":"unknown","message":"nobody is called x","error":{"Unknown":"x"}}
```

A client of a vox-only or sync trait folds the vox error itself:

```rust
let hello = client.greet(name).await.map_err(architect::vox_into_app)?;
```

Without `http_status`, a variant's status comes from its name: `NotFound`
404; `Unauthenticated`, `InvalidCredentials`, `SessionExpired` 401;
`Forbidden`, `PermissionDenied` 403; `Conflict` 409; `Internal` 500;
anything else 400. Errors raised before the method runs are
`422 invalid_body`, `400 invalid_path` and `400 invalid_query`.

## Code tour

1. `crates/architect-derive/src/lib.rs` `error`, `expand_error`,
   `default_status`: the derives, the `HttpError` impl, the transport variant.
2. `crates/architect/src/lib.rs` `TransportError` (`detail`, `retryable`) and
   `RepoError`, the entity repos' error.
3. `crates/architect/src/lib.rs` `ClientError` (`app`, `is_infrastructure`,
   `is_retryable`, `into_app`) and `vox_into_app`.
4. `crates/architect/src/http.rs` `HttpError`, `ErrorBody`: the trait the
   envelope is built from, and the envelope.
5. `crates/architect/src/http.rs` `respond`, `err`: the server side.
6. `crates/architect/src/http.rs` `HttpClient`: a non-2xx reply decodes as
   `ErrorBody<E>` into `ClientError::App`; anything else is `Transport`,
   retryable on 5xx.

The handler returns `Err(GreetError::Unknown(..))`. On HTTP, `err` takes the
status from the `HttpError` impl and writes `ErrorBody { code, message, error }`;
the HTTP client decodes `App(e)` and the generated trait impl returns `e`. On
vox, the error arrives as the call's user error and `vox_into_app` returns
it; a dropped connection becomes `GreetError::Transport(..)`, with vox's
judgment of whether a retry can help.

## Tests to read

- `crates/architect/tests/http_routes.rs`: a `#[architect::error]` enum, the
  `invalid_path` and `invalid_query` codes, a typed `NotFound` on every face.
- `crates/architect/tests/http_bytes.rs`: the typed error as a 404 on a byte route.
- `crates/architect/tests/http_face.rs`: `"code":"unknown"` and
  `422 invalid_body` from a hand-written `HttpError`.
- `examples/greeter/tests/round_trip.rs`: the same `GreetError::Unknown` on
  every wire.

## Status

Done in architect. One gap: an action projected from a service
(`#[architect::service(actions)]`) fails with the error's `Display`, not the
HTTP face's `{code, message}`.

- **Task** does not use `#[architect::error]` or `vox_into_app` yet; it has its
  own client error types (for example `task:features/files/files-client/src/lib.rs`).
- **Studio** does not use them yet.
