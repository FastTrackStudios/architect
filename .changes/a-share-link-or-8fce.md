---
"architect-desktop" = "patch"
"architect-sites" = "patch"
"vault-obsidian" = "patch"
---

A share link or site build no longer follows symlinks: the vault's walk lists no symlink, inside the vault or out of it, and walks into no symlinked folder, so a link to a file outside the vault, or to a private folder or .git inside it, is never listed, hashed, built, copied or served. Files are copied and zipped through an opener that follows no link (vault::obsidian::open_own), so a file swapped for a symlink after the build is left out, and a link's byte route serves only files under its built output. A symlinked note or attachment no longer appears in the app's vault either.
