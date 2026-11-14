---
"xtask" = "minor"
"architect-mobile" = "patch"
---

Changed `cargo xtask dist desktop` and `dist ios` to build the app with its `wasi` feature, so installed WASI component extensions run in release builds (Cranelift on the desk, Pulley on iOS); `--no-wasi` leaves Wasmtime out. The standard distribution crate gains a `wasi` feature, and `apps/mobile/ios/build-app.sh` no longer requires the sandbox's WASI Counter for a `--features wasi` build (#707).
