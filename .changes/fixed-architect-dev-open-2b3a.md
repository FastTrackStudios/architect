---
"architect-desktop" = "patch"
"architect-extensions" = "patch"
---

Fixed `architect dev --open <file>` when the file's opener is a development copy's: it waits (up to 20 s) for that copy's program to start and say what it opens, instead of opening the file as a note
