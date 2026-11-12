---
"architect-app" = "patch"
"architect-mobile" = "patch"
---

Added the Architect app on TestFlight: apps/mobile builds for an iPhone or iPad (architect-app no longer fails to compile for iOS: tokio for FILES_* there too), with its app icon, launch screen and App Store keys (ITSAppUsesNonExemptEncryption false, NSLocalNetworkUsageDescription); apps/mobile/ios/deploy-testflight.sh signs and uploads it (--dry-run validates only), and the release's ios job ships every tag
