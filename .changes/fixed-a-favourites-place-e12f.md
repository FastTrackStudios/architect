---
"architect-extensions" = "patch"
"architect-favourites" = "patch"
---

Fixed a favourites place that a browser would read as another site (a backslash, %, :, ?, @, //) being accepted (architect_favourites::place_ok), and a [[favourites]] declaration that breaks a rule now drops that declaration (Manifest::favourites_dropped, reported by EXT_CHECK) instead of making the whole manifest unreadable, which held back an installed extension's share layers
