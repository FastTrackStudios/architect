---
"crdt" = "minor"
---

Presence carries the host's clock: DocPresence::now, each PresenceDriver pings it into its peer's SharedClock, Here::play_now stamps a playhead in it, so a follower in another process reads the leader where it is
