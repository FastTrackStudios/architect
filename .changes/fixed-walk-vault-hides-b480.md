---
"vault-obsidian" = "patch"
---

Fixed `walk_vault` listing dot-files and dot-folders (`.empty-folders`, `.DS_Store`): they are hidden anywhere in the vault, as vault-live's walker hides them. Fixed `frontmatter_write::write_frontmatter` writing an added empty key as `key: null`; it writes `key:`, as Obsidian does (#442).
