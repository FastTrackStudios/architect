---
"architect-web" = "minor"
"xtask" = "minor"
---

Added `cargo xtask dist <desktop|node|node-image|web|web-image|all> [--manifest-path <crate>]`: builds a distribution of Architect (a crate whose `main` is `architect_desktop::run(..)`, the Architect app by default) through the release's own scripts and images, which now take the distribution's identity (`scripts/desktop/linux.sh` and `macos.sh` `APP_*`, `scripts/web-bundle.sh` `ARCHITECT_DISTRIBUTION`, `flake.nix` `ARCHITECT_IMAGE_NAME` / `ARCHITECT_IMAGE_BIN`); the release workflow builds the Architect app through it. Added `/app/distribution.json` to the web app (`architect_web::Distribution`): it takes the distribution's name and lists the extensions compiled in. Guide: Your own Architect.
