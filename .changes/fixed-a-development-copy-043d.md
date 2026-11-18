---
"architect-app" = "patch"
---

Fixed a development copy's views sometimes not being pane kinds (`WORKSPACE_OPEN` answering "no view") after it loaded: the Vault page reads the extensions' views in its render, kept while their kinds and titles are the same, in place of a memo that could be left behind by the last load
