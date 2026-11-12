---
"architect-extensions" = "patch"
---

Fixed a misshapen [[favourites]] declaration (a misspelt key, an unknown target) failing the whole extension manifest, which held back a node's shared files: it is dropped and reported like one that breaks a rule
