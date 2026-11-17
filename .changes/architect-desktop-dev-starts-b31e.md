---
"architect-app" = "minor"
"architect-desktop" = "minor"
"architect-distribution" = "minor"
---

architect-desktop dev starts an isolated Architect: its own settings, vault (the demo vault, builtin:demo), control socket and file secrets, with only the extensions asked for (--extension, --only, --bare) and a note open (--open); dev list, dev stop, dev path. ARCHITECT_DEV=1 does it from the environment. Distribution::folder_env names the settings folder's variable.
