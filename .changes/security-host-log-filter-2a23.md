---
"architect" = "minor"
---

Security: `host::log_filter` now returns a per-layer filter, the env filter ANDed with `host::writes_credentials`, so no `RUST_LOG` directive (`tungstenite::protocol=trace` included) can make a host log the WebSocket frames that carry a bearer. Migration: a host that installed it with `fmt().with_env_filter(log_filter(env))` uses `registry().with(fmt::layer().with_filter(log_filter(env)))`.
