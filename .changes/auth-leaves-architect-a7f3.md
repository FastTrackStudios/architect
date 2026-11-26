---
"architect-accounts" = "patch"
"xtask" = "minor"
---

Auth is a repository of its own (FastTrackStudios/auth): `features/auth`, `apps/auth-server` and `apps/mock-oauth` leave this workspace, and the workspace takes `architect-auth`, `auth`, `auth-proto`, `auth-db`, `auth-client` and `auth-server` from it at one rev. Auth's wire types derive their own traits and its rows go through its own `auth-macros` entity, so it no longer depends on Architect. `just mock-oauth` and `just auth-demo` run from a checkout of it (`LOCAL_STACK_AUTH_PATH`, else `../auth`); its browser tests and the `auth-ui` sheet live there.
