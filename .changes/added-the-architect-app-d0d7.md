---
"architect-desktop" = "minor"
---

Added the Architect app headless: `architect-desktop serve` runs the app's node with no window (`architect_desktop::node::start`). It joins the account as a `server` device, serves its documents over iroh, holds the vaults in `ARCHITECT_VAULTS` as a sync peer (`VaultCatalog` + `crdt::peer::pull`), builds `ARCHITECT_SITES`, and serves `/health`, `/node/status`, `/vox` and (with a bearer) `/mcp` (`NODE_STATUS`, `NOTES_LIST`, `NOTES_READ`, `NOTES_WRITE`). New feature `gui` (default) is the window; `--no-default-features --features vox` builds the node alone. `flake.nix` builds its OCI image, and a release tag publishes `ghcr.io/fasttrackstudios/architect` with the binary attached.
