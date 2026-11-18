---
"architect-extensions" = "patch"
---

Fixed a process copy of a linked extension's `Extension::files` not reaching its host: the guest sends them beside its contribution (`contrib::FILES_KEY`, read past by older hosts), the host keeps them (`Contributions::runtime_file`, `check_files`), and its toolbars read images from them when its folder has no such file
