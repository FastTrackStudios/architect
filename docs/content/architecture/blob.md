+++
title = "Blob transfer and storage"
description = "Resumable, sha256-verified fetch into place, a content-addressed blob store, and blobs by hash on demand from peers and servers: streamed, pinned, collected."
weight = 63
+++

`architect::blob` (feature `blob`, native only) is the byte lane: getting a
file from somewhere else onto this device without corrupt or half-written
files, keeping blobs by their hash, and fetching the blobs a document
points at from another device or a server when they are read.

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

## Blobs by hash, on demand: `Fetcher`

A [document](@/architecture/documents.md) holds its audio and images by
sha256 (its kind's `DocKind::blobs` reads them out). On another device the
document opens at once; a blob is fetched when it is read.

```rust,ignore
use architect::blob::{Fetcher, HttpRemote, LocalFs, VoxRemote};

let fetcher = Fetcher::new(LocalFs::new(data_dir.join("blobs")))
    .with_remote(Arc::new(VoxRemote::new(peer_client)))     // the peer the song came from
    .with_remote(Arc::new(HttpRemote::new(server_blobs)));  // the org server's copy

let blob = fetcher.open_at(hash, seek_to).await?;  // streams as it arrives
fetcher.fetch(hash).await?;                         // or: wait until it is whole
```

| | |
|---|---|
| Remotes | Anything that implements `Remote` (a blob's size by hash, its bytes from an offset), tried in order. `VoxRemote` reads a peer's `BlobTransfer` service over any vox link (iroh, WebSocket, in-process); `HttpRemote` sends `GET <base>/<hex>` with a `Range`; a `LocalFs` (a mounted drive) is one too. |
| One transfer | However many readers open the same hash, one transfer runs; `progress(hash)` watches it (`Progress`). |
| Resume | A transfer is a `ResumableFetch` into `<root>/.partial/<hex>`: a dropped link keeps the `.part`; the next fetch, from the same remote or the next one, asks for the rest. After a reconnect, `set_remotes` swaps the lane. |
| Streamed | `open` / `open_at` hand back a reader at once; it yields bytes as they land, so playback starts before the end arrives. |
| Verified | The whole blob is hashed before it moves to `<root>/<hex[0..2]>/<hex>`. Streamed bytes are checked only then: a corrupt transfer that restarts ends a reader with `InvalidData`. |

A transfer runs on its own task: dropping the reader does not stop it.

### Serving

A node serves its store to peers with `BlobTransferHost`, mounted like any
vox service (behind the lane's permissions gate); `.allow(|hash| ..)`
narrows which blobs it sends. A server's HTTP route answers with
`serve_http(&store, &headers, &hash)`, ranged by
[`architect::http::ranged`](@/architecture/http.md), which a browser's
`<audio>` seeks against too.

```rust,ignore
router.with(blob_transfer_service_descriptor(), BlobTransferDispatcher::new(
    BlobTransferHost::new(Arc::new(store.clone())).allow(move |hash| shared.contains(&hash)),
));
```

## Keep offline, collect the rest

- `LocalFs::pin(hash)` keeps a blob whatever points at it (`<root>/.pins/<hex>`,
  an empty file); `unpin`, `is_pinned`, `pinned`.
  `crdt::blobs::keep_offline(kind, doc, &fetcher)` pins and fetches every
  blob a document points at; `crdt::blobs::release` unpins them.
- `LocalFs::collect(&live, grace)` removes every blob that is not in `live`
  and not pinned, and abandoned partial fetches; `live` is
  `crdt::blobs::live(&kinds, docs)` over every document on the device.
  Anything younger than `grace` stays, so a blob stored since the set was
  computed survives.

## Not yet

- The wasm side: an OPFS file plus an `IndexedDB` ledger for the resume
  state, as Signal's web packs keep today. It is a follow-up.
- Fetching from several remotes at once (ranges split across peers); a
  transfer reads one remote at a time and falls through on failure.
- Task's Files platform (BLAKE3 chunks) is a separate store today;
  [documents](@/architecture/documents.md) plans one store for both.

Extracted from Signal's content client and pack downloads and the Task
server's attachment object store (STA-388).
