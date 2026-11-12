---
"architect-app" = "minor"
"architect-commands" = "minor"
---

Added the signed-in account's org vaults to the Architect app: each org gets a vault in the app folder (`Org vaults/<slug>`, kept relative so an iOS update finds it), mounted in the navigator at once and synced with the org's node over its lane (on sign-in, every two minutes and on Sync now; both sides' changes kept). A phone's left drawer lists them with their sync state and the org switcher. The app's vault is remembered relative to the app folder and found again after iOS moves it. Added `SHELL_SIDEBAR_TOGGLE`.
