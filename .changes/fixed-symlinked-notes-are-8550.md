---
"architect-desktop" = "patch"
"vault-live" = "patch"
"vault-obsidian" = "patch"
"vault-sync-client" = "patch"
---

Fixed: symlinked notes are no longer listed or read by sync or MCP either: vault-live's walk, the sync manifest and get_file (behind NOTES_LIST, NOTES_READ, the doc peer and vault-collab) list no symlink and read through none (vault_live::own::open_own, which vault::obsidian::open_own now re-exports), and put_file, delete_file, move_file, set_folder and open_collab refuse a path through one. Nothing is deleted: the watcher reports no Delete for a symlink, nor for a path that was one when it went (a link removed, as stow -D does), so a note that is now hidden keeps its file on this node and on every peer. vault-sync-client's write_file also refuses an absolute path, which it wrote outside the local copy before.
