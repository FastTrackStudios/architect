---
"architect-workspace" = "patch"
---

APP_SHOT resolves the window's document before painting it, so a shot between a change and the next frame no longer panics on a removed element (invalid SlotMap key, blitz-paint) and takes the app down
