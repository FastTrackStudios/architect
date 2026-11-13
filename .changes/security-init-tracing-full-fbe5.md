---
"architect-telemetry" = "minor"
---

Security: init_tracing_full (and init_tracing) installs the same credential cap as architect::host::log_filter: no RUST_LOG directive lets DEBUG or TRACE from tungstenite or tokio_tungstenite through, whose WebSocket frames carry bearers. New: capped_registry, writes_credentials, CREDENTIAL_LOGGING_TARGETS.
