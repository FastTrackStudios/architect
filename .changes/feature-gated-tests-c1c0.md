---
"architect-extensions" = "none"
"architect-workspace" = "none"
"xtask" = "none"
---

Main CI runs the tests a feature keeps out of `cargo nextest run --workspace`: `cargo xtask feature-tests` finds each one (a `[[test]]`'s `required-features`, a test behind `cfg(feature …)`) and fails on any no pass in `feature-tests.toml` runs; `--run` runs the passes. The WASI extension tests fail in CI instead of skipping, and `wasi_web` finds node in `devenv --profile wasi shell` (now the current node, which has JSPI). `architect-workspace`'s canvas-size tests run natively without `paint-web`.
