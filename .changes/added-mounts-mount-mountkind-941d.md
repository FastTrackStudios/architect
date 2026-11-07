---
"architect-navigator" = "minor"
---

Added `Mounts`, `Mount`, `MountKind`, `Place` and `OrgBadge`: one navigator over every org's vault and the providers, drawn as a sectioned left list (top rows, Recent Files/, Vaults/ with org badges and wiki marks, Assets/, Tags/); `NavigatorSource::mounts`, `VaultIndex::mounts`/`mount_of`/`in_vault`, `NavigatorState::badge`/`is_wiki`/`mount_icon`, `ViewState::sections_folded`, `tree::subtree`. A folder with a folder note draws a file icon, underlined on hover (#443)
