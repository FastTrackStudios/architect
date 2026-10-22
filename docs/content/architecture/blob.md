+++
title = "Blob transfer and storage"
description = "Resumable, sha256-verified fetch into place, and a content-addressed blob store."
weight = 63
+++

`architect::blob` (feature `blob`, native only) is the byte lane: getting a
file from somewhere else onto this device without corrupt or half-written
files, and keeping blobs by their hash.

## Fetch one file: `ResumableFetch`

```rust,ignore
use architect::blob::{ResumableFetch, Sha256, Source, join_under};

// `rel` comes from a host or a manifest: keep it inside the library.
let dest = join_under(&library_root, &rel)?;
let job = ResumableFetch::new(dest, size).sha256(Sha256::from_hex(&hex)?);
if !job.is_here() {
    let done = job.run(&source, |bytes| report(bytes, size)).await?;
}
```

`source` is anything that implements `Source`: given an offset, it returns
the file's bytes from there to the end (an HTTP `Range: bytes=<offset>-`
request, a vox read call that takes an offset, a file on a mounted drive).

| Step | What happens |
|---|---|
| Already here? | `<dest>` has the expected size and `<dest>.sha256` (`"<size>:<hex>\n"`) matches it and the expected hash: return at once, the source is never called. |
| One at a time | A second fetch of the same path in this process fails with `BlobError::Busy`. |
| Resume | Bytes go to `<dest>.part`, opened for append. The bytes already there are hashed and the source is asked for the rest from that offset. A `.part` longer than the file is stale and starts again. |
| Verify | The whole file is hashed in one pass. A wrong hash removes the `.part` and the fetch starts again from zero, up to `attempts` times (default 2). |
| Into place | The `.part` is synced and renamed over `<dest>`, then the sidecar is written. |

Errors leave the right thing behind: a source that breaks off
(`BlobError::Source`) or ends early (`Incomplete`) keeps the `.part` so the
next run resumes, while a wrong hash (`HashMismatch`) or a source sending too
much (`TooLong`) removes it. Dropping the future pauses the fetch and keeps
the `.part`.

## Store blobs by hash: `BlobStore` and `LocalFs`

```rust,ignore
use architect::blob::{BlobStore, LocalFs};

let store = LocalFs::new(data_dir.join("blobs"));
let hash = store.put(Some(claimed), Box::pin(upload_body)).await?;
if store.has(hash).await? {
    let blob = store.get(hash).await?; // blob.len, blob.reader (AsyncRead)
}
```

A blob's identity is its sha256, so putting the same bytes twice is a no-op.
`put` streams the body into `<root>/.tmp/` while hashing it, refuses it if
the caller's expected hash differs, and renames it to
`<root>/<hex[0..2]>/<hex>`. When the expected hash is already stored, the
body is not read at all. `BlobStore` is object-safe, so an app can hold an
`Arc<dyn BlobStore>` and add other backends (S3, Nextcloud) later.

## Not yet

- The wasm side: an OPFS file plus an `IndexedDB` ledger for the resume
  state, as Signal's web packs keep today. It is a follow-up.
- A ready-made HTTP `Source`. Wrap your client's `Range` request in a
  `Source` impl for now.

Extracted from Signal's content client and pack downloads and the Task
server's attachment object store (STA-388).
