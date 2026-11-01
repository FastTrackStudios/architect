+++
title = "Blobs"
description = "architect::blob: content-addressed bytes, fetched by hash on demand from a peer or a server, streamed while they arrive, resumed, pinned and collected."
weight = 25
+++

## What it is and when to use it

`architect::blob` is the byte lane. `LocalFs` is a content-addressed store
(files by SHA-256). `Fetcher` gets a blob by hash on demand from an ordered
list of remotes (a peer over vox or iroh, a server over HTTP), streams it to
the reader while bytes arrive, runs one transfer per hash however many ask,
and resumes from the partial file after a dropped link. Pins keep what must
be offline; `collect` removes what no document references. `ResumableFetch`
is the single-file form: a resumable, hash-verified download into place.

Use it for audio, images and any file a document points at: the document
holds the hash and `DocKind::blobs` says which hashes it holds; the bytes
never go into the Loro document. Design: [Blobs](@/architecture/blob.md).

## A minimal example

Feature `blob` (native); `vox` adds the peer lane, `http-client` the HTTP
remote, `http` the server route. After `features/crdt/crdt/tests/blobs.rs`:

```rust
use architect::blob::{BlobTransferDispatcher, BlobTransferHost, Fetcher, HttpRemote,
                      LocalFs, VoxRemote, blob_transfer_service_descriptor};

// The device that has the take: store it, serve it
let studio = LocalFs::new(dir.join("blobs"));
let hash = studio.put_bytes(&take).await?;      // write hash.to_hex() into the document
let router = LayerRouter::new().with(
    blob_transfer_service_descriptor(),
    BlobTransferDispatcher::new(BlobTransferHost::new(Arc::new(studio.clone()))),
);

// Another device: fetch on demand, peer first, then the server
let fetcher = Fetcher::new(LocalFs::new(dir.join("laptop")))
    .with_remote(Arc::new(VoxRemote::new(blob_client)))
    .with_remote(Arc::new(HttpRemote::new("https://server/blobs/")));
let blob = fetcher.open_at(hash, 0).await?;      // blob.reader streams as it lands
fetcher.fetch(hash).await?;                      // or wait for all of it

crdt::blobs::keep_offline(&kind, &doc, &fetcher).await?;            // pin and fetch a document's
fetcher.store().collect(&crdt::blobs::live(&kinds, docs), grace).await?;
```

A server answers `HttpRemote` with `serve_http(&store, &headers, &hash)` on
its blob route; `BlobTransferHost::allow` limits which hashes a host serves.

## Code tour

1. `crates/architect/src/blob.rs` `Sha256`, `Source`, `ResumableFetch`: append
   to a `.part`, hash, rename into place, write a `.sha256` sidecar.
2. `crates/architect/src/blob.rs` `BlobStore` (`put`, `get`, `has`,
   `get_from`) and `LocalFs` (`put_bytes`, `path_for`).
3. `crates/architect/src/blob/fetch.rs` `Fetcher` (`with_remote`, `fetch`,
   `open`, `open_at`, `progress`), `Remote`, `Progress`.
4. `crates/architect/src/blob/wire.rs` `BlobTransfer` (`size`, `read` into a
   channel in 256 KiB chunks), `BlobTransferHost`, `VoxRemote`.
5. `crates/architect/src/blob/http_remote.rs` `HttpRemote` (`GET` with
   `Range`) and `crates/architect/src/blob/http_serve.rs` `serve_http`.
6. `crates/architect/src/blob/keep.rs` `LocalFs::pin`, `unpin`, `pinned`,
   `collect`.
7. `features/crdt/crdt/src/blobs.rs` `referenced`, `live`, `missing`,
   `keep_offline`, `release`: blobs as documents see them.

`open_at` reads from disk when the store already has the hash. Otherwise it
joins or starts the one transfer for that hash: the fetcher walks its
remotes, asks each for the size, and runs a `ResumableFetch` into the
partial file, reading the remote from the partial's length. Meanwhile the
reader is fed from the growing file. At the end the whole blob is hashed and
moved into place; a mismatch ends readers with an `InvalidData` error.

## Tests to read

- `crates/architect/tests/blob.rs` (feature `blob`): an interrupted download
  resumes from the part; the sidecar skips a file already here; a wrong hash
  is removed and retried; one transfer per file; a dropped fetch frees its slot.
- `features/crdt/crdt/tests/blobs.rs`: a song's audio arrives on demand,
  streamed, and resumes after a dropped link; blobs over iroh; a server's copy
  over HTTP resumes with a range; a host refuses what it does not allow; a
  pinned blob survives collection; `keep_offline` fetches and pins a song's audio.

## Status

Done in architect; `architect-files` stores its bytes in a `LocalFs`. Not yet:
the browser side (OPFS and an IndexedDB ledger); fetching from several
remotes at once (it falls through one at a time); verifying streamed bytes
before the end.

- **Task** runs iroh-blobs with BLAKE3 for its Files; moving to one store with
  `architect::blob` is planned.
- **Studio** also runs iroh-blobs; its sessions' media is still to move onto
  hashes in the document and a `Fetcher`.
