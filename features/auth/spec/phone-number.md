+++
title = "Phone number"
description = "Rules for phone-number verification and sign-in."
weight = 76
+++

# Phone Number

r[auth.phone.send]
The phone-number plugin MUST issue short-lived verification codes for
valid E.164 phone numbers.

r[auth.phone.provider]
SMS delivery MUST go through a provider abstraction with test and
fail-closed modes.

r[auth.phone.verify]
Verification with a valid code MUST mark the phone number as verified
for the matching user, or create a phone-only user when no user owns the
number.

r[auth.phone.expiry]
Expired, unknown, or already-consumed phone verification codes MUST NOT
verify a phone number.

r[auth.phone.duplicate]
Updating a phone number MUST reject numbers already owned by another
user.

r[auth.phone.update]
An authenticated user MUST be able to update their phone number; the new
number starts unverified until a code is verified.

r[auth.phone.signin]
Phone verification MAY create a session so phone-number authentication
can be used as a sign-in flow.

r[auth.phone.plugin-descriptor]
The phone-number capability MUST be represented by an Architect auth
plugin descriptor with a stable id, upstream parity target,
dependencies, and capability names.

r[auth.phone.plugin-routes]
The phone-number plugin descriptor MUST own send-code, verify-code, and
update-phone route metadata used by transport and OpenAPI adapters.
