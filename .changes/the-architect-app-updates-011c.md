---
"architect" = "minor"
"architect-distribution" = "minor"
"architect-updates" = "minor"
"xtask" = "minor"
---

The Architect app updates itself (#523): Updates (features/updates, UPDATES_* actions, Settings → Updates, a quiet status item) checks at start and daily, installs on Install after verifying the org-signed SHA256SUMS, into the Nix launcher's ARCHITECT_UPDATE_DIR or over a writable APPIMAGE, else reports the download page; unsigned releases are reported, never installed. The release build stamps its version (ARCHITECT_RELEASE_VERSION via cargo xtask dist --version); an unstamped build is a development build that never offers updates. architect::release gains Updater::find, fetch_verified, persist_executable, write_atomic, staging_in, ORG_KEY and Candidate::notes_url (a new field: a struct literal of Candidate needs it).
