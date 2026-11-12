---
"architect-accounts" = "major"
---

Changed sign-in to the system browser: `Accounts::sign_in` and `sign_in_browser` open the issuer's page (iOS: `ASWebAuthenticationSession`; a desk: the default browser, back on a loopback port) and sign in by authorization code + PKCE, the token traded for an issuer session (`Issuer::redeem_code`, `trade_for_session`; `redirect`, `browser`). The device-code flow is `sign_in_with_code`. The sign-in sheet no longer takes an email and password; `ACCOUNT_SIGN_IN` signs in in the browser.
