---
"architect-distribution" = "minor"
"architect-app" = "minor"
---

Added `Distribution::shipped(folder)`: a distribution names a folder of extension packages it ships beside its binary, and the app loads each `<binary's folder>/<folder>/<id>/` that holds an `extension.toml` as a built-in when it starts (`Distribution::shipped_folders`, `Distribution::shipped_packages`). For what cannot be linked in: a process extension, a program of its own that ships with the app. A package that does not read is left out with a warning; a folder that is not there ships none.
