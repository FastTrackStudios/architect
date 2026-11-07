---
"architect-desktop" = "minor"
"architect-files" = "minor"
---

Added the S3 face over synced roots (architect-files feature `s3`: `sync::s3::S3Face`, `S3Keys`): a bucket per org, keys `Projects/<project>/…` and `Assets/<kind>/…`, PutObject and multipart uploads saved as versions, ListObjectVersions over a path's history, SigV4 keys per person or device checked by role; the node serves it on `s3_bind` (`ARCHITECT_S3_BIND`) and issues keys with `architect-desktop serve s3-key`. Added `Engine::save_content`.
