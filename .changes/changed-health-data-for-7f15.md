---
"architect-extensions" = "patch"
---

Changed health data for extensions to the app's own compiled-in extensions only: an installed or development extension's `health_read` / `health_write` is refused (`device.health`, `device::HEALTH_LINKED_ONLY`); the iOS app's phone build is signed with its own entitlements (`apps/mobile/ios/Architect.entitlements`: HealthKit without clinical records or background delivery) within the profile, checked by `apps/mobile/ios/sign-entitlements.py`
