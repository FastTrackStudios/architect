---
"architect-navigator" = "minor"
"architect-navigator-extension" = "minor"
"vault" = "minor"
---

Added VaultSource::resource_url (a vault path or an embed's bare name as a file: URL) and VaultSource::write (FileOp::CreateFolder, Move and SetProperty done on disk, with their events), architect_navigator::vault_source::note_from_text, NavigatorMount's columns and on_new_note, and OpenVault::create_folder, move_file and set_property. VaultSource now gives a note's feature image as a vault path, and its word count.
