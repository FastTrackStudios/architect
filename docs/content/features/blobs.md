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

8. `crates/architect/src/blob/s3.rs` `S3Store` (feature `blob-s3`):
   `open`, `holder_id`, `put_reader`, `delete`, `verify`, `list`,
   `key_for`, `with_events`; `S3Config`; `HolderEvents`.

`open_at` reads from disk when the store already has the hash. Otherwise it
joins or starts the one transfer for that hash: the fetcher walks its
remotes, asks each for the size, and runs a `ResumableFetch` into the
partial file, reading the remote from the partial's length. Meanwhile the
reader is fed from the growing file. At the end the whole blob is hashed and
moved into place; a mismatch ends readers with an `InvalidData` error.

### In an S3 bucket

`S3Store` (feature `blob-s3`) keeps blobs in any S3-compatible service:
Garage, MinIO, Cloudflare R2, AWS S3, Ceph RGW. It is a `BlobStore` (a
`FilesServer` or an app can store in it directly) and a `Remote` (a
`Fetcher` fills its `LocalFs` from it, resuming by range, as from a peer).

- **Keys are hashes:** `<prefix>/sha256/<hh>/<hex>`. Nodes sharing a bucket
  name the same bytes with the same key, so they dedup across each other;
  a put of a hash the bucket holds sends nothing.
- **A put never stages locally.** The body streams to a temporary key in
  8 MiB parts while it is hashed; only bytes that hash to what was
  expected are copied to their key, server-side.
- **A holder with a stable id.** `holder_id` (`s3:<uuid>`) is kept in the
  bucket (`<prefix>/architect-holder`), so every node opening it agrees.
  Put, delete and verify (a scrub: read back whole and hashed) are the
  only writes and checks, each reported to the store's `HolderEvents`:
  the hook for replica accounting.
- **Credentials** are the caller's to supply (`S3Config`); its `Debug`
  redacts the secret.

```rust
let store = S3Store::open(&S3Config {
    endpoint: Some("http://garage.lan:3900".into()),
    region: "garage".into(),
    bucket: "architect-blobs".into(),
    prefix: "architect".into(),
    access_key_id, secret_access_key, virtual_hosted: false,
}).await?;
let hash = store.put(None, body).await?;            // deduped by sha256
let fetcher = Fetcher::new(LocalFs::new(dir)).with_remote(Arc::new(store));
```

## Tests to read

- `crates/architect/tests/blob.rs` (feature `blob`): an interrupted download
  resumes from the part; the sidecar skips a file already here; a wrong hash
  is removed and retried; one transfer per file; a dropped fetch frees its slot.
- `features/crdt/crdt/tests/blobs.rs`: a song's audio arrives on demand,
  streamed, and resumes after a dropped link; blobs over iroh; a server's copy
  over HTTP resumes with a range; a host refuses what it does not allow; a
  pinned blob survives collection; `keep_offline` fetches and pins a song's audio.
- `crates/architect/tests/blob_s3.rs` (feature `blob-s3`, a real S3 server
  in process, `s3s-fs`): a 20 MB put in parts reads back by range; two
  handles on one bucket are one holder and store a hash once; bytes that
  hash wrong are refused; a scrub finds a corrupted object; a `Fetcher`
  fills itself from the bucket; the holder events.

## Status

Done in architect; `architect-files` stores its bytes in a `LocalFs`, and the
node also in an S3 bucket (`blob_s3_bucket`). Not yet:
the browser side (OPFS and an IndexedDB ledger); fetching from several
remotes at once (it falls through one at a time); verifying streamed bytes
before the end.

- **Task** runs iroh-blobs with BLAKE3 for its Files; moving to one store with
  `architect::blob` is planned.
- **Studio** also runs iroh-blobs; its sessions' media is still to move onto
  hashes in the document and a `Fetcher`.
