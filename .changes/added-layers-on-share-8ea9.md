---
"architect-accounts" = "minor"
"architect-desktop" = "minor"
"architect-distribution" = "minor"
"architect-extensions" = "minor"
"architect-sites" = "minor"
"architect-spaces" = "minor"
---

Added layers on share links (#528): a share and each audience carry `listen` (`Listen`: page, reference, full; a new link plays to reference, a share written before reads as full), `ShareChange::Listen` and `ShareLinks::audience_listen` set it; extensions place a note type's files with `[[share_layers]]` (`ShareLayersDecl`, `architect_sites::InstalledLayers`, `Distribution::share_layers`, `node::start_with_layers`); the node answers a file above a link's layer with 403 and `x-architect-layer`, by any spelling of it on disk, and leaves it out of the files page, the archive and the queue; `<link>_files.json` lists what a link serves with each file's size, sha256, layer and URL; `SHARE_AUDIENCE_UPDATE` and `listen` on `SHARE_CREATE`, `SHARE_UPDATE` and `SHARE_AUDIENCE_ADD`; the Share sheet shows how far each link and audience plays.
