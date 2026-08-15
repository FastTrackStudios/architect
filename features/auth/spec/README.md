+++
title = "Auth specs"
description = "Implementation contracts for architect-auth capabilities."
weight = 1
+++

# Auth specs

These files define the behavior `architect-auth` must provide while
porting from the FastTrackStudios `better-auth` fork into typed
Architect entities, repos, and service commands.

Every normative rule uses a stable `r[...]` identifier so later
implementation and verification annotations can prove coverage with
Tracey.

## Capability Specs

- [Core model](core.md)
- [Email/password](email-password.md)
- [Anonymous users](anonymous.md)
- [Sessions](sessions.md)
- [Custom session](custom-session.md)
- [Additional fields](additional-fields.md)
- [OpenAPI](open-api.md)
- [Error taxonomy](error-taxonomy.md)
- [Password management](passwords.md)
- [Email verification](email-verification.md)
- [Email OTP](email-otp.md)
- [Magic link](magic-link.md)
- [OAuth and account linking](oauth.md)
- [OAuth proxy](oauth-proxy.md)
- [One Tap](one-tap.md)
- [One-time token](one-time-token.md)
- [Multi-session](multi-session.md)
- [Last login method](last-login-method.md)
- [Username](username.md)
- [Phone number](phone-number.md)
- [SIWE](siwe.md)
- [HaveIBeenPwned](haveibeenpwned.md)
- [MCP](mcp.md)
- [JWT](jwt.md)
- [OIDC provider](oidc-provider.md)
- [Organizations and RBAC](organizations.md)
- [Two-factor authentication](two-factor.md)
- [Passkeys and WebAuthn](passkeys.md)
- [Device authorization](device-authorization.md)
- [API keys](api-keys.md)
- [Bearer tokens](bearer.md)
- [CAPTCHA](captcha.md)
- [Admin and user management](admin.md)
- [Transport integrations](transports.md)
- [Storage and migrations](storage.md)
- [Security audit](security.md)
- [Test utilities](test-utils.md)
