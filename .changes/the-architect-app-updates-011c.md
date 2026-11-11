---
"architect" = "minor"
"architect-distribution" = "minor"
"architect-updates" = "minor"
"xtask" = "minor"
---

The Architect app updates itself (#523): Updates (features/updates, UPDATES_* actions, Settings → Updates, a quiet status item) checks at start and daily, installs on Install (destructive: an outside caller waits for a person's yes) after verifying the org-signed SHA256SUMS, into the Nix launcher's ARCHITECT_UPDATE_DIR or over a writable APPIMAGE that is the image it runs from, else reports the download page; unsigned releases are reported, never installed. The release build stamps its version (ARCHITECT_RELEASE_VERSION via cargo xtask dist --version, which accepts only X.Y.Z or X.Y.Z-nightly.YYYYMMDD); an unstamped build is a development build that never offers updates. architect::release gains Updater::find, fetch_verified, persist_executable, write_atomic, staging_in, sweep_staging, ORG_KEY, Updater::limits and Candidate::notes_url; downloads are capped (1 GiB asset, 1 MiB sums and signature, ReleaseError::TooLarge), give up after 60 s without a byte instead of 30 min overall, and stage 0600 until verified (a new field: a struct literal of Candidate needs it).
