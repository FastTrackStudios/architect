//! Capability tokens: short, opaque, Ed25519-signed, expiring grants.
//!
//! A capability says "the holder may do `purpose` to `subject` until
//! `expires_unix`". The server mints it with its [`ServerKeypair`]; any
//! holder of the matching [`VerifyingKey`] can check it. Typical uses are
//! signed URLs (a blob upload or download, a read under a path prefix)
//! where the grant has to travel in a query string rather than a session.
//!
//! # Wire format
//!
//! ```text
//! token   = base64url_nopad( signature[64] || message )
//! message = "cap/v1|" purpose "|" expires_unix "|" subject
//! ```
//!
//! `purpose` is a short tag (`[a-z0-9._-]`, 1–64 bytes). `expires_unix`
//! is decimal seconds since the epoch. `subject` is any UTF-8 string
//! with no control characters, and it may contain `|`, because it is the
//! last field. The signature covers the exact message bytes, so the
//! message is also re-encoded after parsing and has to match byte for
//! byte. That leaves one valid encoding per grant.
//!
//! # Fail-closed verification
//!
//! [`Capability::verify`] returns `Ok` only when the signature, the
//! version, the purpose the caller expects, and the expiry all check
//! out. Anything else is an error: a tampered byte, another key, a
//! token minted for a different purpose, a token at or past its
//! expiry, an oversized token or malformed base64. The checks run in
//! that order, so nothing in an unauthenticated message is parsed
//! before its signature is verified.
//!
//! Verification uses `verify_strict`, which rejects weak public keys and
//! malleable signatures.
//!
//! # Key file
//!
//! [`ServerKeypair::load_or_generate`] stores the raw 32-byte Ed25519 seed
//! with no envelope. On unix the file mode (`0600`) is the trust
//! boundary: the file is created `0600` at `open(2)`, so it is never
//! readable by others, even briefly. Loading a key file that is group- or
//! world-accessible fails with [`CapabilityError::InsecureKeyFile`]
//! instead of trusting a key that may have leaked.
//!
//! Native only: the key file needs a filesystem and the OS random source.
//!
//! Extracted from the Task server's `capability.rs` and
//! `attachments/signed_url.rs` (STA-389). The purpose there was an
//! `up`/`dn`/`md` enum. Here it is a caller-chosen tag that `verify`
//! requires to match.

use std::fmt;
use std::io::{Read as _, Write as _};
use std::path::Path;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ed25519_dalek::{SECRET_KEY_LENGTH, SIGNATURE_LENGTH, Signature, Signer as _, SigningKey};
use zeroize::Zeroize as _;

pub use ed25519_dalek::VerifyingKey;

/// Version tag: the first field of every signed message.
pub const TOKEN_VERSION: &str = "cap/v1";

/// Longest token string [`Capability::verify`] will decode. Longer input
/// is refused before any base64 or signature work.
pub const MAX_TOKEN_LEN: usize = 4096;

/// Longest allowed purpose tag, in bytes.
pub const MAX_PURPOSE_LEN: usize = 64;

/// Why a capability could not be issued, verified, or keyed.
#[derive(Debug, thiserror::Error)]
pub enum CapabilityError {
    /// The token is not valid unpadded base64url.
    #[error("token base64 invalid")]
    BadBase64,
    /// The signature does not verify under this key (tampered, or minted
    /// by another key).
    #[error("token signature invalid")]
    BadSignature,
    /// `now >= expires_unix`.
    #[error("token expired")]
    Expired,
    /// The token is well formed and signed, but for another purpose.
    #[error("token purpose mismatch")]
    WrongPurpose,
    /// The token's structure is wrong (too short, too long, bad fields,
    /// or a non-canonical encoding).
    #[error("token malformed")]
    Malformed,
    /// The message does not start with [`TOKEN_VERSION`].
    #[error("unsupported token version")]
    UnsupportedVersion,
    /// A field given to [`Capability::issue`] cannot be encoded.
    #[error("invalid {0}")]
    InvalidField(&'static str),
    /// The key file is group- or world-accessible (unix mode shown).
    #[error("key file is accessible to other users (mode {mode:o}); expected 0600")]
    InsecureKeyFile {
        /// The file's permission bits.
        mode: u32,
    },
    /// Reading or writing the key file failed.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The key file's contents are not a key, or the OS random source failed.
    #[error("keypair: {0}")]
    Keypair(String),
}

/// One grant: `purpose` on `subject`, valid while `now < expires_unix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capability {
    /// What the holder may do: a short tag such as `blob.up`.
    pub purpose: String,
    /// Unix seconds at which the grant stops being valid (exclusive).
    pub expires_unix: i64,
    /// What it applies to: an id, a content hash, a path prefix.
    pub subject: String,
}

impl Capability {
    /// A grant that expires at `expires_unix`.
    #[must_use]
    pub fn new(purpose: impl Into<String>, subject: impl Into<String>, expires_unix: i64) -> Self {
        Self {
            purpose: purpose.into(),
            expires_unix,
            subject: subject.into(),
        }
    }

    /// A grant valid for `ttl_secs` from `now_unix`. Fails rather than
    /// wrapping on overflow.
    pub fn expiring_in(
        purpose: impl Into<String>,
        subject: impl Into<String>,
        now_unix: i64,
        ttl_secs: i64,
    ) -> Result<Self, CapabilityError> {
        let expires_unix = now_unix
            .checked_add(ttl_secs)
            .ok_or(CapabilityError::InvalidField("expiry"))?;
        Ok(Self::new(purpose, subject, expires_unix))
    }

    fn canonical_message(&self) -> String {
        format!(
            "{TOKEN_VERSION}|{}|{}|{}",
            self.purpose, self.expires_unix, self.subject
        )
    }

    fn check_fields(&self) -> Result<(), CapabilityError> {
        if !valid_purpose(&self.purpose) {
            return Err(CapabilityError::InvalidField("purpose"));
        }
        if self.subject.chars().any(char::is_control) {
            return Err(CapabilityError::InvalidField("subject"));
        }
        Ok(())
    }

    /// Sign and encode. The returned string is opaque to clients.
    pub fn issue(&self, kp: &ServerKeypair) -> Result<String, CapabilityError> {
        self.check_fields()?;
        let msg = self.canonical_message();
        let sig: Signature = kp.signing.sign(msg.as_bytes());
        let mut out = Vec::with_capacity(SIGNATURE_LENGTH.saturating_add(msg.len()));
        out.extend_from_slice(&sig.to_bytes());
        out.extend_from_slice(msg.as_bytes());
        let token = URL_SAFE_NO_PAD.encode(out);
        // Never mint a token `verify` would refuse.
        if token.len() > MAX_TOKEN_LEN {
            return Err(CapabilityError::InvalidField("subject"));
        }
        Ok(token)
    }

    /// Verify `token` for `expected_purpose` at time `now_unix`.
    ///
    /// Fails closed: `Ok` only if the signature, version, purpose and
    /// expiry all check out (see the module docs for the order).
    pub fn verify(
        token: &str,
        expected_purpose: &str,
        key: &VerifyingKey,
        now_unix: i64,
    ) -> Result<Self, CapabilityError> {
        if token.len() > MAX_TOKEN_LEN {
            return Err(CapabilityError::Malformed);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| CapabilityError::BadBase64)?;
        let (sig_bytes, msg_bytes) = bytes
            .split_first_chunk::<SIGNATURE_LENGTH>()
            .ok_or(CapabilityError::Malformed)?;
        let sig = Signature::from_bytes(sig_bytes);
        key.verify_strict(msg_bytes, &sig)
            .map_err(|_| CapabilityError::BadSignature)?;

        // Authenticated from here on.
        let msg = std::str::from_utf8(msg_bytes).map_err(|_| CapabilityError::Malformed)?;
        let parsed = Self::parse_message(msg)?;
        if parsed.purpose != expected_purpose {
            return Err(CapabilityError::WrongPurpose);
        }
        if now_unix >= parsed.expires_unix {
            return Err(CapabilityError::Expired);
        }
        Ok(parsed)
    }

    /// [`Capability::verify`] against the system clock.
    pub fn verify_now(
        token: &str,
        expected_purpose: &str,
        key: &VerifyingKey,
    ) -> Result<Self, CapabilityError> {
        Self::verify(token, expected_purpose, key, unix_now()?)
    }

    fn parse_message(s: &str) -> Result<Self, CapabilityError> {
        let mut parts = s.splitn(4, '|');
        if parts.next() != Some(TOKEN_VERSION) {
            return Err(CapabilityError::UnsupportedVersion);
        }
        let purpose = parts.next().ok_or(CapabilityError::Malformed)?;
        let expires_unix: i64 = parts
            .next()
            .ok_or(CapabilityError::Malformed)?
            .parse()
            .map_err(|_| CapabilityError::Malformed)?;
        let subject = parts.next().ok_or(CapabilityError::Malformed)?;
        let cap = Self::new(purpose, subject, expires_unix);
        // One encoding per grant: reject `+5`, `05`, bad tags, etc.
        if cap.check_fields().is_err() || cap.canonical_message() != s {
            return Err(CapabilityError::Malformed);
        }
        Ok(cap)
    }
}

fn valid_purpose(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= MAX_PURPOSE_LEN
        && p.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

/// Seconds since the Unix epoch, from the system clock.
pub fn unix_now() -> Result<i64, CapabilityError> {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| CapabilityError::Keypair("system clock before 1970".into()))?
        .as_secs();
    i64::try_from(secs).map_err(|_| CapabilityError::Keypair("system clock out of range".into()))
}

/// The server's Ed25519 signing key. The seed is zeroized on drop.
/// `Debug` shows only the public key.
#[derive(Clone)]
pub struct ServerKeypair {
    signing: SigningKey,
}

impl fmt::Debug for ServerKeypair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServerKeypair")
            .field("verifying_key", &self.signing.verifying_key())
            .finish_non_exhaustive()
    }
}

impl ServerKeypair {
    /// A fresh key from the OS random source. Not persisted.
    pub fn generate() -> Result<Self, CapabilityError> {
        let mut seed = [0u8; SECRET_KEY_LENGTH];
        getrandom::fill(&mut seed).map_err(|e| CapabilityError::Keypair(format!("os rng: {e}")))?;
        let kp = Self {
            signing: SigningKey::from_bytes(&seed),
        };
        seed.zeroize();
        Ok(kp)
    }

    /// Load the 32-byte seed at `path`, or generate one and write it there
    /// with mode `0600` if the file does not exist.
    ///
    /// Fails closed: a file of the wrong length, or (on unix) one with any
    /// group/other permission bit set, is an error, never a new key.
    pub fn load_or_generate(path: &Path) -> Result<Self, CapabilityError> {
        match Self::load(path) {
            Err(CapabilityError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {}
            other => return other,
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let kp = Self::generate()?;
        match create_private(path) {
            Ok(mut file) => {
                let mut seed = kp.signing.to_bytes();
                let written = file.write_all(&seed).and_then(|()| file.sync_all());
                seed.zeroize();
                if let Err(e) = written {
                    let _ = std::fs::remove_file(path);
                    return Err(e.into());
                }
                Ok(kp)
            }
            // Lost a race with another process creating it: use theirs.
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Self::load(path),
            Err(e) => Err(e.into()),
        }
    }

    /// Load an existing key file (see [`Self::load_or_generate`]).
    pub fn load(path: &Path) -> Result<Self, CapabilityError> {
        let file = std::fs::File::open(path)?;
        // Check the permissions of the file actually opened, so a swap of the
        // path between the check and the read changes nothing.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = file.metadata()?.permissions().mode() & 0o777;
            if mode & 0o077 != 0 {
                return Err(CapabilityError::InsecureKeyFile { mode });
            }
        }
        let mut buf = Vec::with_capacity(SECRET_KEY_LENGTH.saturating_add(1));
        // Read at most one byte past a key, so a huge file is not slurped.
        let limit = u64::try_from(SECRET_KEY_LENGTH.saturating_add(1)).unwrap_or(u64::MAX);
        file.take(limit).read_to_end(&mut buf)?;
        let result = <[u8; SECRET_KEY_LENGTH]>::try_from(buf.as_slice())
            .map(|seed| Self {
                signing: SigningKey::from_bytes(&seed),
            })
            .map_err(|_| CapabilityError::Keypair(format!("key file wrong length: {}", buf.len())));
        buf.zeroize();
        result
    }

    /// The public half, which is all a verifier needs.
    #[must_use]
    pub fn verifying_key(&self) -> VerifyingKey {
        self.signing.verifying_key()
    }

    /// Sign `cap` with this key. Same as [`Capability::issue`].
    pub fn issue(&self, cap: &Capability) -> Result<String, CapabilityError> {
        cap.issue(self)
    }

    /// Verify `token` for `expected_purpose` under this key's public half.
    pub fn verify(
        &self,
        token: &str,
        expected_purpose: &str,
        now_unix: i64,
    ) -> Result<Capability, CapabilityError> {
        Capability::verify(token, expected_purpose, &self.verifying_key(), now_unix)
    }
}

#[cfg(unix)]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}
