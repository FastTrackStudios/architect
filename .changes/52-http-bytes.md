---
"architect" = "minor"
"architect-rpc-derive" = "minor"
---

HTTP-only service methods for bytes: a `BodyStream` argument (a streamed upload capped at the route's `limit`), a `RequestHead` argument (method, URI, headers) and a `ByteBody` result (total, content type, windowed reader) served through `http::ranged::serve_body`, so every byte route answers `Range` with `206` / `416`. A trait with such a method has no vox face; its `Service` token still mounts through `provide_http`. Media, blob and upload routes no longer need hand-written axum handlers.
