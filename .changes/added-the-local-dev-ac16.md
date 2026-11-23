---
"auth-server" = "minor"
"xtask" = "minor"
---

Added the local dev stack's automatic sign-in: on the demo seed the web app and the desktop window come up signed in as `ada@local.test`. The auth server's `AUTH_DEV_AUTO_SIGN_IN=<email>` (`ServerConfig::dev_auto_sign_in`, `auth_server::dev::auto_sign_in`) signs that account in at `/oauth2/authorize` instead of showing the form, refused at boot in a release build, off loopback, for a non-demo account or off the stack's state; the web app's `/app/config.json` `"auto_sign_in": true` (`Config::auto_sign_in`, `architect_web::auto_sign_in`) starts the sign-in by itself once a page load; a development window (`ARCHITECT_DEV_INSTANCE`) with `ARCHITECT_DEV_SIGN_IN=<email>` and `ARCHITECT_DEV_PASSWORD_FILE=<file>` at a loopback issuer signs in with no active account (`architect_desktop::dev::sign_in_from_env`, `architect_app::set_dev_sign_in`); `cargo xtask dev env app` prints those variables, and the stack's guard refuses an auto sign-in with a non-demo account or a non-loopback issuer
