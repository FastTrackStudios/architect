---
"architect-extensions" = "minor"
---

Added WASI components on iOS: architect-extensions' wasi runner interprets them with Wasmtime's Pulley there (no JIT), a 64 MiB memory reservation with no guard, traps found without signals; the same limits and compile cache. New: feature pulley (the interpreter anywhere), wasi::PULLEY, wasi::PULLEY_RESERVATION.
