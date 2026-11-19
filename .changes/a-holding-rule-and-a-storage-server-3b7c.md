---
"architect-files" = "minor"
"architect-desktop" = "minor"
---

Added a holding rule per device (#904): `architect_files::HoldRule` (`paths`, `skip`, `max`; `hold::glob` with `*`, `?`, `**`) on `EngineConfig::hold`. A file the rule leaves out stays in the manifest and is never fetched or held; `RootStatus::left_out` and `Engine::left_out` count it apart from `missing`, and `Engine::named_keeping` leaves it out of what a node copies to its bucket. The node reads it from `files_hold_paths`, `files_skip_paths`, `files_hold_max` (`ARCHITECT_FILES_HOLD_PATHS`, `_SKIP_PATHS`, `_HOLD_MAX`; `Config::hold_rule`) and narrows its bucket with `blob_s3_max`, `blob_s3_skip_paths` (`ARCHITECT_BLOB_S3_MAX`, `_SKIP_PATHS`; `Config::bucket_rule`); `/node/status` has `files_store.left_out`. Added `architect-desktop serve register [--issuer <url>] [--session-file <path>] [--name <name>]`: a machine with no browser is signed in to the account with a code its person approves elsewhere, and its session is written for the node (`node::register::sign_in`, `write_session`), to run as a storage server (`ARCHITECT_PLATFORM=storage`).
