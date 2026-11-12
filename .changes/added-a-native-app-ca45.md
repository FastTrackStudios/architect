---
"auth" = "minor"
"auth-server" = "minor"
---

Added a native app sign-in to the issuer: a registered loopback redirect (`http://127.0.0.1/…` or `[::1]`) matches any port (RFC 8252 §7.3, never `localhost`), and `/oauth2/token` takes the token-exchange grant (`TOKEN_EXCHANGE`, `ExchangeForSession`): a fresh access token issued to a client with the new `issue_sessions` flag (`OidcClientConfig::issue_sessions`, `issue_sessions` in `AUTH_OIDC_CLIENTS`) traded once, within `EXCHANGE_WINDOW_SECONDS`, for a new issuer session. Discovery lists the grant.
