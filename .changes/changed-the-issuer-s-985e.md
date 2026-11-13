---
"auth" = "minor"
---

Changed the issuer's `whoami` and `list_organizations` to accept its own OIDC access token only from a client registered `first_party` (new `OidcClientConfig::first_party`, `"first_party"` in `AUTH_OIDC_CLIENTS`, default `false`; `skip_consent` is only a consent convenience and no longer grants this), when it verifies with the default audience, carries `openid` and belongs to a sign-in in force. Set `first_party` on every client whose access token lists orgs or calls `whoami` (the web app's `architect`; Task's `task`, which mirrors memberships this way). Security: the device calls (`register_device`, `list_devices`, `forget_device`) take a session token alone and refuse an access token: Task's in-app device sync, which passes its access token to `register_device` and `forget_device`, stops working (accepted: Task is being sunset; Task's `peer` CLI uses a session and is unaffected).
