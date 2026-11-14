---
"architect-extensions" = "minor"
---

Added `platforms` to `extension.toml` for every kind (#578): off those operating systems an extension is listed with why ("Linux only: not on macOS": `LoadError::NotHere`, `browse::Row::not_here`, `EXT_LIST` `platforms` and `not_here`, `Preview::runs_on`), never run (a linked one's `build` included), its data kept; the store refuses it (`InstallError::NotHere`), a `[requires]` on its service names it, and `Host::set_platform` checks against another platform
