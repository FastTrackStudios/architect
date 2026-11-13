---
"architect-desktop" = "minor"
---

Security: the org lane re-checks an open live note's bearer every `lane_recheck_secs` (`ARCHITECT_LANE_RECHECK_SECS`, default 30, also how long a bearer is trusted, was 60) and ends the session once it no longer opens the note (signed out, revoked, expired, not a member); a WebSocket upgrade from a browser on another origin is refused unless listed in `lane_origins` (`ARCHITECT_LANE_ORIGINS`). The node serves `Notes::create` on the org lane.
