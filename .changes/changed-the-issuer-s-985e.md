---
"auth" = "minor"
---

Changed the issuer's `whoami` and `list_organizations` to accept its own OIDC access token only when it verifies with the default audience, names an enabled first-party client (`skip_consent`) and carries `openid`; Security: the device calls (`register_device`, `list_devices`, `forget_device`) take a session token alone and refuse an access token.
