---
"architect-desktop" = "patch"
"architect-distribution" = "patch"
"architect-web" = "patch"
"xtask" = "patch"
---

Fixed the local stack (`just dev plain`): the web app (`apps/web`) now runs under `dx serve --platform web` on 7413, rebuilt on a save, and signs in at the local issuer; and an app run on a folder of its own (`ARCHITECT_CONFIG_DIR`) claims its one instance under that folder too (`Distribution::instance_id`), so a development copy opens beside an installed Architect that is running instead of handing its launch to it and ending.
