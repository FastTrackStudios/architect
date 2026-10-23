---
"architect" = "minor"
---

`iroh_link::connect` now dials a restarted peer by id. Before, the dial ran into iroh's own timeout whenever the dialling endpoint still had a connection to the peer from before it restarted. The whole dial is now bounded by `DIAL_TIMEOUT` (10 s). If the first attempt fails, it closes the stale connections to that peer, runs address lookup again and retries once. A failure now carries a typed `DialError` inside the `io::Error`, read back with `DialError::of`, so callers can tell `PeerUnreachable` from `Timeout` and `Failed`. New: `connect_with`, `DialOptions` and `IrohLinkSource::with_dial_options`.
