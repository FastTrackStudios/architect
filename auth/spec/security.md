+++
title = "Security audit"
description = "Rules for production threat modeling and hardening review."
weight = 145
+++

# Security Audit

r[auth.security.threat-model]
The library MUST keep a reviewed threat model covering sessions, cookies,
OAuth state, token storage, passkeys, 2FA, organization permissions, API
keys, admin impersonation, and service-to-service RPC.

r[auth.security.audit]
The production hardening audit MUST map identified threats to concrete
controls and evidence.

r[auth.security.hardening-checklist]
The audit MUST include a production hardening checklist for secrets,
cookies, rate limits, replay prevention, audit logging, dependency
review, and operational deployment settings.

r[auth.security.findings]
Critical audit findings MUST be treated as release-blocking and must have
linked issues before 1.0 completion.
