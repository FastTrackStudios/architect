---
"architect-app" = "minor"
"architect-capture" = "minor"
"architect-shell" = "minor"
---

Added the phone's capture button from Task (#445): architect-shell's Action::on_swipe_up (SwipeUp, SWIPE_UP) lifts the round separate button with the pointer and lets go into its swipe, and Action::with_badge puts a count on it; architect-capture's use_to_process counts the captures waiting; the Architect app's capture button starts a timer on a slide up (TIME_START) and carries the count
