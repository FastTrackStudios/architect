+++
title = "Email OTP"
description = "Rules for email one-time password authentication."
weight = 99
+++

# Email OTP

r[auth.emailotp.send]
Email OTP send MUST create a hashed one-time verification value for the
canonical email address.

r[auth.emailotp.verify]
Email OTP verification MUST validate the submitted code against the stored
hash and canonical email address.

r[auth.emailotp.expiry]
Expired OTP values MUST fail verification.

r[auth.emailotp.resend-limit]
OTP sends SHOULD be rate-limited per email address.

r[auth.emailotp.single-use]
OTP values MUST be single-use.

r[auth.emailotp.session]
Successful OTP verification MAY create a session when requested.

r[auth.emailotp.test-sink]
The Rust API MUST expose the generated OTP value for deterministic tests and
local email-sink integration.

r[auth.emailotp.plugin-descriptor]
The email OTP plugin MUST declare its Better Auth lineage, verification-token
dependency, and capabilities for send, verify, expiration, rate limiting,
single-use codes, test sink, and session creation.

r[auth.emailotp.plugin-routes]
The email OTP plugin routes MUST be generated from the shared auth command
catalog so Rust, Axum, Vox, and OpenAPI expose the same operation ids and
session requirements.
