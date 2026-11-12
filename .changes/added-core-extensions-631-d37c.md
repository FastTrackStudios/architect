---
"xtask" = "minor"
---

Added core extensions (#631): `extensions/core-extensions.toml` lists by reference the extensions the standard release of Architect includes (id, publisher, source, version, tag, commit, the publisher's signature, and a `linked` or `packaged` form per platform), and `dist/standard` builds that release outside the workspace, generated from the list, with every architect crate patched back to the checkout so one Architect is compiled. First entries: Studio's setlists (linked, v26.11.4) and Studio's share-page island (packaged, from Studio's signed release). New `cargo xtask core check|sync|verify|fetch|graph`; `cargo xtask dist --features`.
