---
"architect-mobile" = "minor"
"xtask" = "minor"
---

Added `cargo xtask dist ios` and `ios-sim`: a distribution crate's iPhone and iPad app (`apps/mobile/ios/build-app.sh --manifest-path`), and `dist/standard` built for iOS with its linked core extensions (`ios` in `extensions/core-extensions.toml`; Studio's setlists and the new `studio-views` entry); `deploy-testflight.sh` takes `DIST_MANIFEST` and `--build-only`, which stops before signing (#640)
