+++
title = "Capability tokens"
description = "Ed25519-signed, expiring purpose|expiry|subject grants, and the 0600 server key file behind them."
weight = 62
+++

`architect::capability` (feature `capability`, native only) mints and checks
short, opaque grants that say "the holder may do *purpose* to *subject* until
*expiry*". Use one where a grant has to travel without a session: a signed
upload or download URL, or a read under a path prefix.

```rust,ignore
use architect::capability::{Capability, ServerKeypair};

let kp = ServerKeypair::load_or_generate(&data_dir.join("server.key"))?;
let token = Capability::expiring_in("blob.dn", content_hash, now, 300)?.issue(&kp)?;
// ...later, in the handler:
let grant = Capability::verify(&token, "blob.dn", &kp.verifying_key(), now)?;
```

## Format

`base64url_nopad(signature[64] || "cap/v1|" purpose "|" expires_unix "|" subject)`.
`purpose` is `[a-z0-9._-]{1,64}`. `subject` is UTF-8 with no control
characters and may contain `|`. A token is at most 4096 characters.

## Fails closed

`verify` returns `Ok` only when all of these pass, in this order:

1. the token is at most `MAX_TOKEN_LEN` characters and is valid base64url;
2. the Ed25519 signature checks out (`verify_strict`);
3. the message has the `cap/v1` version and is the one canonical encoding;
4. the purpose equals the caller's `expected_purpose` (`WrongPurpose`);
5. `now < expires_unix` (`Expired`).

A tampered byte, another server's key, a grant minted for another purpose,
or an expired grant each give a distinct error, and none of them gives a grant.
The purpose check is what stops an upload grant being replayed as a download.
Pick one tag per use and never reuse a tag for two meanings.

## The key file

`ServerKeypair::load_or_generate(path)` stores the raw 32-byte seed. On unix
it is created with mode `0600` at `open(2)`, so other users can never read it,
even briefly. Loading refuses a file with any group or other bit set
(`InsecureKeyFile`) and a file of the wrong length (`Keypair`). It never
replaces an existing file with a new key. The seed is zeroized after use, and
`ServerKeypair`'s `Debug` output shows only the public key.

Rotating the key invalidates every outstanding token. There is no key id in
the format, so to rotate without downtime, verify against the old and the new
`VerifyingKey` for one TTL.

Revocation is by expiry only, so keep TTLs short (minutes).
