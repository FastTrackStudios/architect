---
"architect-accounts" = "minor"
"architect-desktop" = "minor"
"architect-distribution" = "minor"
"architect-extensions" = "minor"
"architect-sites" = "minor"
"architect-spaces" = "minor"
---

Added layers on share links (#528): a share and each audience carry `listen` (`Listen`: page, reference, full; a new link, and a share note without one, plays to reference; an audience never past its share), set with `ShareChange::Listen` and `ShareLinks::audience_listen`. Extensions place a note type's files with `[[share_layers]]` (`ShareLayersDecl`: linear-time, ASCII-case-insensitive globs, bounded by `MAX_PATTERNS`, `MAX_PATTERN_LEN`, `MAX_STARS`); the node takes the highest layer over every extension's rules, holds every attached file back below Full while a manifest does not read (`architect_sites::InstalledLayers`, `Installed`), and keeps an uninstalled extension's rules in its data dir until forgotten (`ShareLinks::kept_layers`, `forget_layers`, `KeptLayers`; actions `SHARE_LAYERS_KEPT`, `SHARE_LAYERS_FORGET`). Compiled-in rules come from `Distribution::share_layers` through `node::start_with_layers`. A file above a link's layer is 403 with `x-architect-layer`, by any spelling of it on disk; anything outside a link is the same 404 as a path that is nowhere. Each link is built for its layer (`architect_sites::Options::hold_back`), so no page, search index, files page, archive or queue names a held-back file. `<link>_files.json` lists what a link serves (size, sha256, layer, URL; `listen`, `max`; 503 while placement is unknown). `SHARE_AUDIENCE_UPDATE` and `listen` on `SHARE_CREATE`, `SHARE_UPDATE` and `SHARE_AUDIENCE_ADD`; the Share sheet shows how far each link and audience plays, and why a change failed.
