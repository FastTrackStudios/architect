---
"architect" = "minor"
"architect-desktop" = "minor"
"architect-files" = "minor"
---

Added `architect::blob::S3Store` (feature `blob-s3`): blobs in any S3-compatible bucket (Garage, MinIO, R2, AWS, Ceph RGW) as a `BlobStore` and a fetch `Remote`, keyed by sha256 so nodes sharing a bucket dedup, with a stable `holder_id` kept in the bucket and put/delete/verify reported to `HolderEvents`. The node keeps the files' bytes there too with `blob_s3_bucket` (`ARCHITECT_BLOB_S3_*`), `blob_placement` `replicate` or `primary` (a local cache of `blob_cache_mb`). Added `Engine::named`.
