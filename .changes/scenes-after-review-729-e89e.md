---
"architect-extensions" = "minor"
"architect-sandbox-sketch" = "minor"
"architect-workspace" = "minor"
---

Scenes after review (#729): the host limits a scene (4 MB, 100k commands, finite coordinates), balances its layers and decodes it off the UI thread; a scene is clipped to its pane unless the extension is granted overlay (capabilities.ui), which draws over the whole app; a pan no longer resizes; a linked paint view is clipped to its pane.
