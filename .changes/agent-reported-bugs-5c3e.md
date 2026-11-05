---
"architect" = "patch"
---

On wasm32, `dispatch::run` answers `DispatchError::TypeMismatch` instead of panicking when a dispatcher hands back another type, as the native one does. The wasm-only `Send` lints (`future_not_send`, `arc_with_non_send_sync`) are allowed, since wasm32 is single-threaded, so `cargo clippy -p architect --target wasm32-unknown-unknown -D warnings` passes.
