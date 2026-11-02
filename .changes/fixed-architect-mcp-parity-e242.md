---
"architect-mcp" = "patch"
---

Fixed `architect_mcp::parity::Parity` failing every socket call when the temp dir is a shared `/tmp`: its scratch socket now sits in a directory of the process's own, since the control server only answers the user who owns the socket's directory
