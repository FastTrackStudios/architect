---
"architect-desktop" = "patch"
---

Fixed architect-desktop links register: a rewritten mimeapps.list keeps its mode (a private 0600 list stays private) instead of taking the umask's (#635)
