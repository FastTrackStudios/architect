---
"architect" = "minor"
---

Added `permissions_gate::caller_bearer`: the bearer the gate admitted the running call with (the call's own `authorization`, else the connection's), for a call held open to re-check. Security: `host::log_filter` (used by `init_tracing` and `boot`) holds `tungstenite` and `tokio_tungstenite` (`CREDENTIAL_LOGGING_TARGETS`) at INFO whatever `RUST_LOG` says, so a host at TRACE no longer logs WebSocket frames that carry a bearer.
