---
"architect-extensions" = "patch"
---

Fixed a remote view's images: its document had no net provider, so every <img> in a process or remote view stayed blank, a data: one included (Keyflow's palette glyphs drew empty in a development copy); it now fetches data: URIs, and nothing else (no file: path, network or bundle asset)
