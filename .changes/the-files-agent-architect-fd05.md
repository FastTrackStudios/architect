---
"architect-desktop" = "minor"
"architect-files" = "minor"
---

The files agent (architect-files feature agent): FILES_* actions and an HTTP face for roots, browse, pin, unpin, history, restore and checkpoints; a WebDAV face over every root a device knows, browsable offline from its manifest, files fetched on open and kept, PUT as a version. The node serves both on loopback, FILES_* over /mcp.
