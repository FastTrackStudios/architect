---
"architect-extensions" = "minor"
"architect-extension-wasi" = "minor"
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-sandbox-wasi-counter" = "none"
"architect-extension-wasi-hazards" = "none"
---

Added WASI component extensions (#167, part 1). architect-extensions: feature `wasi` with `wasi::WasiRunner` (`kind = "wasm"`: Wasmtime with the component model and Cranelift, a store per instance on its own thread, `Limits` on memory and each call's time, a trap starting it again fresh, a compile cache keyed by the component's hash; `with_limits`, `with_cache`, `with_theme`, `with_processes`, `components`), `Components` and `Stats`; the `architect:extension` world (`wit/extension.wit`); `Manifest::wasm` (`[wasm] component`) and `Manifest::component`; `Process::is_local` (a component's views show in `RemoteView`); the manager's `ComponentKind` (`component`). `RemoteView` repaints after every applied frame (a frame no longer waits for the next input to show). New crate `architect-extension-wasi`: a Rust guest for `wasm32-wasip2` (`Extension`, `host`, `refresh`, `export!`). The lab: feature `wasi` (`architect-desktop --features wasi`) runs the WASI Counter (`apps/lab/wasi-counter`). devenv: a `wasi` profile with the `wasm32-wasip2` target.
