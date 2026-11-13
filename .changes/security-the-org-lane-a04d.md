---
"architect-desktop" = "minor"
---

Security: the org lane re-checks an open live note's credential every `lane_recheck_secs` (`ARCHITECT_LANE_RECHECK_SECS`, default 30, also how long a bearer is trusted, was 60) and ends the session once it no longer opens the note (signed out, revoked, expired, not a member). The credential checked is the one the session's call was admitted with, on the connection or with the call; a session with none ends at once. With the issuer unreachable an open note is kept for `lane_issuer_grace_secs` (`ARCHITECT_LANE_ISSUER_GRACE_SECS`, default 300) after its last good check, then ends; new calls are refused meanwhile. A WebSocket upgrade from a browser on another origin is refused unless listed in `lane_origins` (`ARCHITECT_LANE_ORIGINS`). The node serves `Notes::create` on the org lane.
