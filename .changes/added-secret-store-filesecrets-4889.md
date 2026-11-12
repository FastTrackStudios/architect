---
"architect-accounts" = "minor"
"architect-app" = "minor"
"xtask" = "minor"
---

Added `secret_store`, `FileSecrets` and `ARCHITECT_SECRET_STORE` (file:<path>, file, memory, keychain): a development build on macOS or the iOS simulator, the dev stack's app and every test keep sessions in a 0600 file, never the platform keychain, which asked the person at the screen again on every rebuild of an unsigned binary. `ARCHITECT_FORBID_KEYCHAIN=1` (set for every cargo test, nextest and run in the repo by .cargo/config.toml, and for the stack's app) makes `Keychain` panic instead of opening it. Release builds keep the keychain.
