---
"architect-desktop" = "minor"
---

Added signed-in web clients on the headless app's org lane: with `host_org`, `/org/{slug}/vox` also serves `Notes` and the vaults' `DocSync` to an issuer-session bearer, so a web app edits a note live as its role allows (`OrgLane::with_vaults`, `OrgLane::opens`)
