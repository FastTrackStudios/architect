---
"architect-extensions" = "patch"
"architect-workspace" = "patch"
---

Fixed a main-area view (`EXT_VIEW_OPEN` with placement `tab` or `split`, and the workspace's view opener) opening in a dock's group in place of its pane when the dock had the focus: it opens beside the document on show (#442).
