//! Signing (feature `sign`): minisign's formats, written here so the
//! release workflow and a person sign with the same `cargo xtask ext-sign`.
//!
//! A [`SecretKey`] reads an unencrypted minisign secret key file (what
//! `minisign -G -W` writes, and what the org key in the
//! `release-signing` environments is); an encrypted one is refused, with
//! how to make one that is not. Signatures are minisign's prehashed
//! (`ED`, Ed25519 over BLAKE2b-512) with a trusted comment, so
//! `minisign -V` checks them too. [`SecretKey::generate`] makes a
//! throwaway key for tests and a registry's local trials, never for a
//! release.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use blake2::Digest as _;
use ed25519_dalek::{Signer as _, SigningKey};

use crate::{ArchiveError, read_index, trusted_comment, with_signature};

/// The length of a decoded minisign secret key line.
const SECRET_LEN: usize = 158;

type Blake2b256 = blake2::Blake2b<blake2::digest::consts::U32>;

/// A minisign secret key, unencrypted.
pub struct SecretKey {
    keynum: [u8; 8],
    sk: SigningKey,
}

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretKey")
            .field("key_id", &self.key_id())
            .finish_non_exhaustive()
    }
}

fn key_err(e: impl std::fmt::Display) -> ArchiveError {
    ArchiveError::Key(e.to_string())
}

fn checksum(keynum: &[u8], sk: &[u8]) -> Vec<u8> {
    let mut h = Blake2b256::new();
    h.update(b"Ed");
    h.update(keynum);
    h.update(sk);
    h.finalize().to_vec()
}

impl SecretKey {
    /// Reads a minisign secret key file's text (its comment line, then
    /// the base64 line).
    ///
    /// # Errors
    ///
    /// It does not decode, is not Ed25519, is encrypted, or its checksum
    /// fails.
    pub fn parse(text: &str) -> Result<Self, ArchiveError> {
        let line = text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
            .ok_or_else(|| key_err("the file is empty"))?;
        let raw = B64
            .decode(line)
            .map_err(|e| key_err(format!("not a minisign secret key ({e})")))?;
        if raw.len() != SECRET_LEN {
            return Err(key_err("not a minisign secret key (wrong length)"));
        }
        let part = |from: usize, to: usize| raw.get(from..to).unwrap_or_default();
        if part(0, 2) != b"Ed" || part(4, 6) != b"B2" {
            return Err(key_err("not a minisign Ed25519 secret key"));
        }
        if part(2, 4) != [0, 0] {
            return Err(key_err(
                "the key is encrypted (a password); ext-sign takes an unencrypted key, as \
                 `minisign -G -W` writes and the release-signing environment holds",
            ));
        }
        let keynum: [u8; 8] = part(54, 62).try_into().map_err(key_err)?;
        let sk = part(62, 126);
        // minisign 0.12 leaves an unencrypted key's checksum zero; older
        // ones (and `generate`) write it. Either way the public half must
        // match the secret half, below.
        let chk = part(126, 158);
        if chk.iter().any(|b| *b != 0) && checksum(&keynum, sk) != chk {
            return Err(key_err("its checksum does not match: the file is damaged"));
        }
        let seed: [u8; 32] = sk.get(..32).unwrap_or_default().try_into().map_err(key_err)?;
        let signing = SigningKey::from_bytes(&seed);
        if signing.verifying_key().as_bytes() != sk.get(32..).unwrap_or_default() {
            return Err(key_err("its public half does not match its secret half"));
        }
        Ok(Self { keynum, sk: signing })
    }

    /// A new random key: for tests and trials, never a release.
    ///
    /// # Errors
    ///
    /// The system has no randomness to give.
    pub fn generate() -> Result<Self, ArchiveError> {
        let mut seed = [0u8; 32];
        let mut keynum = [0u8; 8];
        getrandom::fill(&mut seed).map_err(key_err)?;
        getrandom::fill(&mut keynum).map_err(key_err)?;
        Ok(Self {
            keynum,
            sk: SigningKey::from_bytes(&seed),
        })
    }

    /// Its key id, as minisign prints it.
    #[must_use]
    pub fn key_id(&self) -> String {
        format!("{:016X}", u64::from_le_bytes(self.keynum))
    }

    /// The base64 line of its `.pub`: what an app trusts.
    #[must_use]
    pub fn public(&self) -> String {
        let mut raw = b"Ed".to_vec();
        raw.extend_from_slice(&self.keynum);
        raw.extend_from_slice(self.sk.verifying_key().as_bytes());
        B64.encode(raw)
    }

    /// Its `.pub` file.
    #[must_use]
    pub fn public_file(&self) -> String {
        format!(
            "untrusted comment: minisign public key {}\n{}\n",
            self.key_id(),
            self.public()
        )
    }

    /// Its secret key file, unencrypted (as `minisign -G -W` writes).
    #[must_use]
    pub fn secret_file(&self) -> String {
        let mut sk = self.sk.to_bytes().to_vec();
        sk.extend_from_slice(self.sk.verifying_key().as_bytes());
        let mut raw = b"Ed".to_vec();
        raw.extend_from_slice(&[0, 0]);
        raw.extend_from_slice(b"B2");
        raw.extend_from_slice(&[0u8; 32 + 8 + 8]);
        raw.extend_from_slice(&self.keynum);
        raw.extend_from_slice(&sk);
        raw.extend_from_slice(&checksum(&self.keynum, &sk));
        format!(
            "untrusted comment: minisign encrypted secret key\n{}\n",
            B64.encode(raw)
        )
    }

    /// A `.minisig` over `msg`, with `comment` as its trusted comment.
    #[must_use]
    pub fn sign(&self, msg: &[u8], comment: &str) -> String {
        let prehash = blake2::Blake2b512::digest(msg);
        let sig = self.sk.sign(&prehash).to_bytes();
        let mut line = b"ED".to_vec();
        line.extend_from_slice(&self.keynum);
        line.extend_from_slice(&sig);
        let mut global = sig.to_vec();
        global.extend_from_slice(comment.as_bytes());
        let global = self.sk.sign(&global).to_bytes();
        format!(
            "untrusted comment: signature from architect ext-sign\n{}\ntrusted comment: {comment}\n{}\n",
            B64.encode(line),
            B64.encode(global)
        )
    }
}

/// `archive` signed with `key`: its index's signature added (in place of
/// any there was), trusted comment `archext <id> <version>`.
///
/// # Errors
///
/// It is not an archive, or has no index.
pub fn sign_archive(archive: &[u8], key: &SecretKey) -> Result<Vec<u8>, ArchiveError> {
    let (index, bytes) = read_index(archive)?;
    let sig = key.sign(&bytes, &trusted_comment(&index.id, &index.version));
    with_signature(archive, &sig)
}
