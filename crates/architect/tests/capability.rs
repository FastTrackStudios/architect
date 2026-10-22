//! `architect::capability` — every way a token can be wrong must fail
//! closed (STA-389 acceptance: tamper, expiry, wrong purpose, wrong key),
//! and the key file is `0600`.

use architect::capability::{
    Capability, CapabilityError, MAX_TOKEN_LEN, ServerKeypair, TOKEN_VERSION,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

const NOW: i64 = 1_800_000_000;

fn kp() -> ServerKeypair {
    ServerKeypair::generate().unwrap()
}

fn grant() -> Capability {
    Capability::expiring_in("blob.dn", "sha256:deadbeef", NOW, 300).unwrap()
}

#[test]
fn round_trips() {
    let kp = kp();
    let tok = grant().issue(&kp).unwrap();
    let back = kp.verify(&tok, "blob.dn", NOW).unwrap();
    assert_eq!(back, grant());
    assert_eq!(back.expires_unix, NOW + 300);
    // A verifier needs only the public key.
    let back = Capability::verify(&tok, "blob.dn", &kp.verifying_key(), NOW + 299).unwrap();
    assert_eq!(back.subject, "sha256:deadbeef");
}

#[test]
fn message_is_purpose_expiry_subject() {
    let kp = kp();
    let tok = Capability::new("blob.up", "a|b", 42).issue(&kp).unwrap();
    let bytes = URL_SAFE_NO_PAD.decode(&tok).unwrap();
    assert_eq!(
        &bytes[64..],
        format!("{TOKEN_VERSION}|blob.up|42|a|b").as_bytes()
    );
    // `|` inside the subject (the last field) survives.
    assert_eq!(kp.verify(&tok, "blob.up", 0).unwrap().subject, "a|b");
}

// ── tamper ─────────────────────────────────────────────────────────────

#[test]
fn every_flipped_byte_is_rejected() {
    let kp = kp();
    let tok = grant().issue(&kp).unwrap();
    let bytes = URL_SAFE_NO_PAD.decode(&tok).unwrap();
    for i in 0..bytes.len() {
        for bit in [0x01u8, 0x80] {
            let mut bad = bytes.clone();
            bad[i] ^= bit;
            let bad = URL_SAFE_NO_PAD.encode(bad);
            assert!(
                matches!(
                    kp.verify(&bad, "blob.dn", NOW),
                    Err(CapabilityError::BadSignature)
                ),
                "flip {bit:#x} at byte {i} was accepted"
            );
        }
    }
}

#[test]
fn extended_expiry_is_rejected() {
    // Re-sign nothing; just splice a later expiry into the message.
    let kp = kp();
    let tok = grant().issue(&kp).unwrap();
    let mut bytes = URL_SAFE_NO_PAD.decode(&tok).unwrap();
    let msg = String::from_utf8(bytes.split_off(64)).unwrap();
    let forged = msg.replace(&(NOW + 300).to_string(), &(NOW + 999_999).to_string());
    bytes.extend_from_slice(forged.as_bytes());
    let bad = URL_SAFE_NO_PAD.encode(bytes);
    assert!(matches!(
        kp.verify(&bad, "blob.dn", NOW),
        Err(CapabilityError::BadSignature)
    ));
}

#[test]
fn truncated_garbage_and_oversized_tokens_are_rejected() {
    let kp = kp();
    let tok = grant().issue(&kp).unwrap();
    assert!(matches!(
        kp.verify("", "blob.dn", NOW),
        Err(CapabilityError::Malformed)
    ));
    assert!(matches!(kp.verify(&tok[..40], "blob.dn", NOW), Err(_)));
    assert!(matches!(
        kp.verify("not base64!", "blob.dn", NOW),
        Err(CapabilityError::BadBase64)
    ));
    let huge = "A".repeat(MAX_TOKEN_LEN + 1);
    assert!(matches!(
        kp.verify(&huge, "blob.dn", NOW),
        Err(CapabilityError::Malformed)
    ));
    // Padded base64 is not the canonical encoding.
    assert!(kp.verify(&format!("{tok}="), "blob.dn", NOW).is_err());
}

// ── expiry ─────────────────────────────────────────────────────────────

#[test]
fn expiry_is_exclusive_and_fails_closed() {
    let kp = kp();
    let tok = grant().issue(&kp).unwrap();
    assert!(kp.verify(&tok, "blob.dn", NOW + 299).is_ok());
    for t in [NOW + 300, NOW + 301, i64::MAX] {
        assert!(matches!(
            kp.verify(&tok, "blob.dn", t),
            Err(CapabilityError::Expired)
        ));
    }
    // Already expired at issue time.
    let dead = Capability::new("blob.dn", "x", NOW).issue(&kp).unwrap();
    assert!(matches!(
        kp.verify(&dead, "blob.dn", NOW),
        Err(CapabilityError::Expired)
    ));
}

#[test]
fn ttl_overflow_is_an_error_not_a_wrap() {
    assert!(Capability::expiring_in("p", "s", i64::MAX, 1).is_err());
}

// ── wrong purpose ──────────────────────────────────────────────────────

#[test]
fn wrong_purpose_is_rejected() {
    let kp = kp();
    let tok = grant().issue(&kp).unwrap();
    for p in ["blob.up", "blob", "blob.dn ", "BLOB.DN", ""] {
        assert!(
            matches!(kp.verify(&tok, p, NOW), Err(CapabilityError::WrongPurpose)),
            "{p:?}"
        );
    }
}

#[test]
fn unencodable_fields_are_refused_at_issue() {
    let kp = kp();
    for p in ["", "a|b", "Up", "has space", &"x".repeat(65)] {
        assert!(matches!(
            Capability::new(p, "s", NOW).issue(&kp),
            Err(CapabilityError::InvalidField("purpose"))
        ));
    }
    assert!(matches!(
        Capability::new("p", "line\nbreak", NOW).issue(&kp),
        Err(CapabilityError::InvalidField("subject"))
    ));
    assert!(
        Capability::new("p", "x".repeat(MAX_TOKEN_LEN), NOW)
            .issue(&kp)
            .is_err()
    );
}

// ── wrong key ──────────────────────────────────────────────────────────

#[test]
fn wrong_key_is_rejected() {
    let a = kp();
    let b = kp();
    let tok = grant().issue(&a).unwrap();
    assert!(matches!(
        b.verify(&tok, "blob.dn", NOW),
        Err(CapabilityError::BadSignature)
    ));
}

#[test]
fn debug_does_not_print_the_seed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("k");
    let kp = ServerKeypair::load_or_generate(&path).unwrap();
    let seed = std::fs::read(&path).unwrap();
    let dbg = format!("{kp:?}");
    assert!(!dbg.contains(&format!("{:?}", seed.as_slice())));
    assert!(!dbg.to_lowercase().contains(&hex(&seed)));
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

// ── key file ───────────────────────────────────────────────────────────

#[test]
fn load_or_generate_persists_and_reloads_the_same_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/dir/server.key");
    let a = ServerKeypair::load_or_generate(&path).unwrap();
    assert_eq!(std::fs::read(&path).unwrap().len(), 32);
    let b = ServerKeypair::load_or_generate(&path).unwrap();
    assert_eq!(a.verifying_key(), b.verifying_key());
    let tok = grant().issue(&a).unwrap();
    assert!(b.verify(&tok, "blob.dn", NOW).is_ok());
}

#[cfg(unix)]
#[test]
fn key_file_is_created_0600() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.key");
    ServerKeypair::load_or_generate(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "mode was {mode:o}");
}

#[cfg(unix)]
#[test]
fn group_or_world_readable_key_file_is_refused() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("server.key");
    ServerKeypair::load_or_generate(&path).unwrap();
    for mode in [0o644, 0o640, 0o604, 0o660] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        match ServerKeypair::load_or_generate(&path) {
            Err(CapabilityError::InsecureKeyFile { mode: m }) => assert_eq!(m, mode),
            other => panic!("mode {mode:o}: expected InsecureKeyFile, got {other:?}"),
        }
    }
}

#[test]
fn wrong_length_key_file_is_an_error_not_a_new_key() {
    let dir = tempfile::tempdir().unwrap();
    for len in [0usize, 31, 33, 4096] {
        let path = dir.path().join(format!("k{len}"));
        std::fs::write(&path, vec![7u8; len]).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        assert!(
            matches!(
                ServerKeypair::load_or_generate(&path),
                Err(CapabilityError::Keypair(_))
            ),
            "len {len}"
        );
        // And the file was not overwritten.
        assert_eq!(std::fs::read(&path).unwrap().len(), len);
    }
}
