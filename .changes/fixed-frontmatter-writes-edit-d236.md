---
"vault-obsidian" = "patch"
---

Fixed frontmatter writes (edit_frontmatter, edit_tags, VAULT_SET_PROPERTY) re-serialising the whole block: vault_obsidian::frontmatter_write::write_frontmatter edits in place, writing only the changed keys and keeping flow lists, block-list indents and quotes
