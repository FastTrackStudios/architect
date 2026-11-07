---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-files" = "minor"
---

Added the files registry and verified copy accounting (architect-files feature sync): every live file of every root a device may see in a local index searched offline (FILES_SEARCH: words, sha:<hex>, org:, root:, kind:; each hit with its holders, verified count and reachability); signed attestations per holder and org on the peer lanes (ledger/<org>/<holder>); scrubs and Possession challenges over 1 MiB Merkle leaves, so only copies with fresh evidence count; per-org and per-root copy policy, with unpin and evict refused below it (EngineError::BelowPolicy); FILES_LOCATIONS, FILES_COPIES, FILES_POLICY_SET, FILES_POLICY_CHECK, FILES_SCRUB, FILES_EVICT, FILES_HOLDERS; node config files_site, files_verify_secs, files_scrub_mib. Hit gains org, size, content, holders, verified, reachable. An S3 bucket a node stores blobs in (architect::blob::S3Store) is a holder of kind s3 through architect_files::sync::BucketHolder (feature s3), attested by the node as it is written and deleted.
