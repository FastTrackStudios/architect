---
"architect-navigator" = "minor"
---

Added `FolderMeta::icon`, `VaultIndex::display_path`, `NavigatorState::note_badge`; file-type icons are on by default with audio, MIDI, video and DAW-session built-ins, and a provider's item and container draw their own icons; a list grouped by folder leaves the folder off its rows. Renamed `FolderOrg` to `FolderMeta` and `folder_orgs` to `folder_meta`. Fixed a panic when the mouse wheel scrolled the navigator (#443)
