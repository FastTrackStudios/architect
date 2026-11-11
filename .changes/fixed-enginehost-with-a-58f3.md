---
"architect" = "patch"
---

Fixed EngineHost with a dav mount: a request for a path no route has now goes through the finish layers (it went to a bare 404, so a loopback node with ARCHITECT_FILES_DIR answered no share link, #541)
