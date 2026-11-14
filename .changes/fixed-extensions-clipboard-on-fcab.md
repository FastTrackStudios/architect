---
"architect-extensions" = "patch"
---

Fixed extensions' clipboard on iOS (#710 H8): `clipboard_read` and `clipboard_write` use the general pasteboard (`UIPasteboard`) there, where there are no clipboard tools to run
