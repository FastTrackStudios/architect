use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum PasswordError {
    #[error("password hashing failed")]
    Hash,
    #[error("password verification failed")]
    Verify,
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("token generation failed")]
    Generate,
}

#[derive(Debug, thiserror::Error)]
pub enum SecretBoxError {
    #[error("secret encryption failed")]
    Encrypt,
    #[error("secret decryption failed")]
    Decrypt,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretKeyMaterial {
    pub id: String,
    pub secret: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SecretKeyRing {
    active: SecretKeyMaterial,
    fallback: Vec<SecretKeyMaterial>,
}

impl SecretKeyRing {
    pub fn new(active_id: impl Into<String>, active_secret: impl Into<String>) -> Self {
        Self {
            active: SecretKeyMaterial {
                id: active_id.into(),
                secret: active_secret.into(),
            },
            fallback: Vec::new(),
        }
    }

    pub fn with_fallback(mut self, key_id: impl Into<String>, secret: impl Into<String>) -> Self {
        self.fallback.push(SecretKeyMaterial {
            id: key_id.into(),
            secret: secret.into(),
        });
        self
    }

    pub fn active_key_id(&self) -> &str {
        &self.active.id
    }

    fn key_for_id(&self, key_id: &str) -> Option<&SecretKeyMaterial> {
        if self.active.id == key_id {
            return Some(&self.active);
        }
        self.fallback.iter().find(|key| key.id == key_id)
    }

    fn all_keys(&self) -> impl Iterator<Item = &SecretKeyMaterial> {
        std::iter::once(&self.active).chain(self.fallback.iter())
    }
}

pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| PasswordError::Hash)
}

pub fn verify_password(password: &str, hash: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(hash).map_err(|_| PasswordError::Verify)?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

// r[impl auth.sessions.token-random]
pub fn generate_token() -> Result<String, TokenError> {
    let mut bytes = [0_u8; 32];
    rand::rngs::OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| TokenError::Generate)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

// r[impl auth.core.no-plaintext-secrets]
// r[impl auth.sessions.token-hash-storage]
pub fn hash_token(secret: &str, token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":");
    hasher.update(token.as_bytes());
    URL_SAFE_NO_PAD.encode(hasher.finalize())
}

// r[impl auth.oauth.token-encryption]
// r[impl auth.twofactor.secret-encryption]
// r[impl auth.core.secret-aead]
// r[impl auth.core.secret-key-ids]
pub fn encrypt_secret(secret: &str, plaintext: &str) -> Result<String, SecretBoxError> {
    encrypt_secret_with_keyring(&SecretKeyRing::new("primary", secret), plaintext)
}

// r[impl auth.core.secret-aead]
// r[impl auth.core.secret-key-ids]
// r[impl auth.core.secret-rotation]
pub fn encrypt_secret_with_keyring(
    keyring: &SecretKeyRing,
    plaintext: &str,
) -> Result<String, SecretBoxError> {
    let mut nonce = [0_u8; 24];
    rand::rngs::OsRng
        .try_fill_bytes(&mut nonce)
        .map_err(|_| SecretBoxError::Encrypt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(&derive_secret_key(
        &keyring.active.secret,
        keyring.active_key_id(),
    )?)
    .map_err(|_| SecretBoxError::Encrypt)?;
    let ciphertext = cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: plaintext.as_bytes(),
                aad: secret_aad(keyring.active_key_id()).as_bytes(),
            },
        )
        .map_err(|_| SecretBoxError::Encrypt)?;
    Ok(format!(
        "v2.{}.{}.{}",
        keyring.active_key_id(),
        URL_SAFE_NO_PAD.encode(nonce),
        URL_SAFE_NO_PAD.encode(ciphertext)
    ))
}

// r[impl auth.twofactor.secret-encryption]
// r[impl auth.core.secret-aead]
// r[impl auth.core.secret-legacy-decrypt]
pub fn decrypt_secret(secret: &str, envelope: &str) -> Result<String, SecretBoxError> {
    decrypt_secret_with_keyring(&SecretKeyRing::new("primary", secret), envelope)
}

// r[impl auth.core.secret-aead]
// r[impl auth.core.secret-key-ids]
// r[impl auth.core.secret-rotation]
// r[impl auth.core.secret-legacy-decrypt]
pub fn decrypt_secret_with_keyring(
    keyring: &SecretKeyRing,
    envelope: &str,
) -> Result<String, SecretBoxError> {
    if envelope.starts_with("v1.") {
        return keyring
            .all_keys()
            .find_map(|key| decrypt_legacy_secret(&key.secret, envelope).ok())
            .ok_or(SecretBoxError::Decrypt);
    }
    let mut parts = envelope.split('.');
    let (Some("v2"), Some(key_id), Some(nonce), Some(ciphertext), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(SecretBoxError::Decrypt);
    };
    let key = keyring.key_for_id(key_id).ok_or(SecretBoxError::Decrypt)?;
    let nonce = URL_SAFE_NO_PAD
        .decode(nonce)
        .map_err(|_| SecretBoxError::Decrypt)?;
    if nonce.len() != 24 {
        return Err(SecretBoxError::Decrypt);
    }
    let ciphertext = URL_SAFE_NO_PAD
        .decode(ciphertext)
        .map_err(|_| SecretBoxError::Decrypt)?;
    let cipher = XChaCha20Poly1305::new_from_slice(
        &derive_secret_key(&key.secret, key_id).map_err(|_| SecretBoxError::Decrypt)?,
    )
    .map_err(|_| SecretBoxError::Decrypt)?;
    let plaintext = cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: secret_aad(key_id).as_bytes(),
            },
        )
        .map_err(|_| SecretBoxError::Decrypt)?;
    String::from_utf8(plaintext).map_err(|_| SecretBoxError::Decrypt)
}

fn derive_secret_key(secret: &str, key_id: &str) -> Result<[u8; 32], SecretBoxError> {
    let hkdf = Hkdf::<Sha256>::new(Some(b"architect-auth-secret-envelope"), secret.as_bytes());
    let mut key = [0_u8; 32];
    hkdf.expand(secret_aad(key_id).as_bytes(), &mut key)
        .map_err(|_| SecretBoxError::Encrypt)?;
    Ok(key)
}

fn secret_aad(key_id: &str) -> String {
    format!("architect-auth:v2:secret-envelope:{key_id}")
}

fn decrypt_legacy_secret(secret: &str, envelope: &str) -> Result<String, SecretBoxError> {
    let mut parts = envelope.split('.');
    let (Some("v1"), Some(nonce), Some(ciphertext), Some(tag), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return Err(SecretBoxError::Decrypt);
    };
    let nonce = URL_SAFE_NO_PAD
        .decode(nonce)
        .map_err(|_| SecretBoxError::Decrypt)?;
    let ciphertext = URL_SAFE_NO_PAD
        .decode(ciphertext)
        .map_err(|_| SecretBoxError::Decrypt)?;
    let tag = URL_SAFE_NO_PAD
        .decode(tag)
        .map_err(|_| SecretBoxError::Decrypt)?;
    if secret_box_tag(secret, &nonce, &ciphertext).as_slice() != tag {
        return Err(SecretBoxError::Decrypt);
    }
    let plaintext = xor_with_secret_stream(secret, &nonce, &ciphertext);
    String::from_utf8(plaintext).map_err(|_| SecretBoxError::Decrypt)
}

fn xor_with_secret_stream(secret: &str, nonce: &[u8], input: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(input.len());
    for (counter, chunk) in input.chunks(32).enumerate() {
        let mut hasher = Sha256::new();
        hasher.update(secret.as_bytes());
        hasher.update(b":secret-box:");
        hasher.update(nonce);
        hasher.update((counter as u64).to_le_bytes());
        let block = hasher.finalize();
        output.extend(chunk.iter().zip(block.iter()).map(|(byte, key)| byte ^ key));
    }
    output
}

fn secret_box_tag(secret: &str, nonce: &[u8], ciphertext: &[u8]) -> Vec<u8> {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    hasher.update(b":secret-box-tag:");
    hasher.update(nonce);
    hasher.update(ciphertext);
    hasher.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use base64::Engine;

    use super::{
        SecretKeyRing, decrypt_secret, decrypt_secret_with_keyring, encrypt_secret,
        encrypt_secret_with_keyring, generate_token, hash_password, hash_token, verify_password,
    };

    // r[verify auth.email.password-hash]
    #[test]
    fn password_hash_verifies_without_storing_plaintext() {
        let hash = hash_password("correct horse battery staple").expect("hash password");

        assert_ne!(hash, "correct horse battery staple");
        assert!(verify_password("correct horse battery staple", &hash).expect("verify password"));
        assert!(!verify_password("wrong", &hash).expect("verify wrong password"));
    }

    // r[verify auth.sessions.token-random]
    #[test]
    fn generated_tokens_are_url_safe_and_distinct() {
        let first = generate_token().expect("generate token");
        let second = generate_token().expect("generate second token");

        assert_ne!(first, second);
        assert!(first.len() >= 40);
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }

    // r[verify auth.core.no-plaintext-secrets]
    // r[verify auth.sessions.token-hash-storage]
    #[test]
    fn token_hash_is_stable_and_secret_scoped() {
        let token = "session-token";

        let hash = hash_token("a-secret-at-least-32-bytes-long", token);

        assert_ne!(hash, token);
        assert_eq!(hash, hash_token("a-secret-at-least-32-bytes-long", token));
        assert_ne!(hash, hash_token("another-secret-at-least-32-bytes", token));
    }

    // r[verify auth.oauth.token-encryption]
    // r[verify auth.twofactor.secret-encryption]
    // r[verify auth.core.secret-aead]
    // r[verify auth.core.secret-key-ids]
    #[test]
    fn secret_box_round_trip_does_not_store_plaintext() {
        let secret = "a-secret-at-least-32-bytes-long";
        let plaintext = "provider-or-totp-secret";

        let envelope = encrypt_secret(secret, plaintext).expect("encrypt secret");

        assert!(envelope.starts_with("v2.primary."));
        assert_ne!(envelope, plaintext);
        assert!(!envelope.contains(plaintext));
        assert_eq!(
            decrypt_secret(secret, &envelope).expect("decrypt secret"),
            plaintext
        );
        assert!(decrypt_secret("another-secret-at-least-32-bytes", &envelope).is_err());
    }

    // r[verify auth.core.secret-aead]
    #[test]
    fn secret_box_detects_tampering() {
        let secret = "a-secret-at-least-32-bytes-long";
        let plaintext = "provider-or-totp-secret";
        let mut envelope = encrypt_secret(secret, plaintext).expect("encrypt secret");

        envelope.push('A');

        assert!(decrypt_secret(secret, &envelope).is_err());
    }

    // r[verify auth.core.secret-rotation]
    #[test]
    fn secret_box_supports_key_rotation() {
        let old = SecretKeyRing::new("old", "old-secret-at-least-32-bytes-long");
        let old_envelope =
            encrypt_secret_with_keyring(&old, "rotated-secret").expect("encrypt with old key");
        let rotated = SecretKeyRing::new("new", "new-secret-at-least-32-bytes-long")
            .with_fallback("old", "old-secret-at-least-32-bytes-long");
        let new_envelope =
            encrypt_secret_with_keyring(&rotated, "rotated-secret").expect("encrypt with new key");

        assert!(new_envelope.starts_with("v2.new."));
        assert_eq!(
            decrypt_secret_with_keyring(&rotated, &old_envelope).expect("decrypt old envelope"),
            "rotated-secret"
        );
        assert_eq!(
            decrypt_secret_with_keyring(&rotated, &new_envelope).expect("decrypt new envelope"),
            "rotated-secret"
        );
        assert!(
            decrypt_secret_with_keyring(
                &SecretKeyRing::new("new", "new-secret-at-least-32-bytes-long"),
                &old_envelope
            )
            .is_err()
        );
    }

    // r[verify auth.core.secret-legacy-decrypt]
    #[test]
    fn secret_box_decrypts_legacy_v1_for_migration() {
        let secret = "a-secret-at-least-32-bytes-long";
        let plaintext = "legacy-secret";
        let nonce = [7_u8; 24];
        let ciphertext = super::xor_with_secret_stream(secret, &nonce, plaintext.as_bytes());
        let tag = super::secret_box_tag(secret, &nonce, &ciphertext);
        let legacy = format!(
            "v1.{}.{}.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(nonce),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(ciphertext),
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tag)
        );

        assert_eq!(
            decrypt_secret(secret, &legacy).expect("decrypt legacy secret"),
            plaintext
        );
    }
}
