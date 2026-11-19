---
"architect-desktop" = "minor"
"architect-accounts" = "minor"
"architect-files" = "minor"
"architect" = "minor"
---

Added a storage server let in by its own key, with no session and nobody at the machine (the `authorized_keys` shape). `architect-desktop serve key` prints the node's public key (`node::public_key`; `iroh_key_file` / `ARCHITECT_IROH_KEY_FILE` keeps a rebuilt machine the same device). A signed-in app declares it with `ACCOUNT_DEVICE_AUTHORISE {public_key, name, orgs, host}` (`Accounts::authorise_device`), sees it in `ACCOUNT_DEVICES` under `authorised` (`AuthorisedRow`: platform `storage`, its orgs, who authorised it and where) and withdraws it with `ACCOUNT_DEVICE_REVOKE` (`Accounts::revoke_device`, `Accounts::authorised_devices`). The machine runs with `enrol_at` and `enrol_orgs` and no issuer: it lets that node in as those orgs' (`/node/status` `hosted_orgs`, `HostedOrg`) and syncs. A storage device (`node::org::storage_role`, `is_storage`) opens its orgs' file roots and ledgers and no note, and any device of an org is sent only bytes a manifest of that org names (`architect_files::sync::Engine::blob_host_for`). In `architect`: `blob::BlobTransfer::read_range` and `blob::Remote::read_range` (a reader of one range tells the sender where to stop; `Fetcher::relay_range`), and a peer's stream is always drained, so a reader that leaves early no longer breaks the connection other transfers share.

Migration: `blob::BlobTransfer` has a new method, `read_range`: a hand-written implementation adds it, and a gate's permit table names it (`architect_files::sync::BLOB_PERMITS` does). `node::files::more` takes the caller's orgs.
