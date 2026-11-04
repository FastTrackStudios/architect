//! The `.archext` archive: a good one opens; an unsigned, tampered or
//! wrong-key one is refused with the error that says which; a process
//! extension's builds land per platform.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::arithmetic_side_effects
)]

use base64::Engine as _;
use architect_extension_archive::sign::{SecretKey, sign_archive};
use architect_extension_archive::{
    ArchiveError, INDEX, SIGNATURE, Trust, entries, open, pack, registry, write_entries,
};

const MANIFEST: &str = r#"id = "acme.tool"
name = "Tool"
version = "1.2.0"
kind = "process"

[entry]
path = "bin/tool"
"#;

const LUAU: &str = r#"id = "acme.click"
name = "Click"
version = "0.3.0"
kind = "luau"
"#;

/// A package folder: a process extension with two platforms' builds, a
/// page and an asset.
fn package() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path();
    std::fs::write(p.join("extension.toml"), MANIFEST).unwrap();
    for (triple, body) in [
        ("x86_64-unknown-linux-gnu", "linux x86_64"),
        ("aarch64-apple-darwin", "mac arm64"),
    ] {
        std::fs::create_dir_all(p.join("bin").join(triple)).unwrap();
        std::fs::write(p.join("bin").join(triple).join("tool"), body).unwrap();
    }
    std::fs::create_dir_all(p.join("pages")).unwrap();
    std::fs::write(p.join("pages/home.md"), "# Tool\n").unwrap();
    std::fs::write(p.join("icon.svg"), "<svg/>").unwrap();
    dir
}

fn signed(key: &SecretKey) -> Vec<u8> {
    let dir = package();
    let (_, archive) = pack(dir.path()).unwrap();
    sign_archive(&archive, key).unwrap()
}

fn trusting(key: &SecretKey) -> Trust {
    Trust::none().with("test key", key.public())
}

#[test]
fn a_good_archive_opens_and_extracts_this_platforms_build() {
    let key = SecretKey::generate().unwrap();
    let archive = signed(&key);
    let v = open(&archive, &trusting(&key)).unwrap();
    assert_eq!(v.index.id, "acme.tool");
    assert_eq!(v.index.version, "1.2.0");
    assert_eq!(
        v.index.platforms,
        ["aarch64-apple-darwin", "x86_64-unknown-linux-gnu"]
    );
    assert_eq!(v.signer, "test key");
    let out = tempfile::tempdir().unwrap();
    v.extract_for(out.path(), &["aarch64-apple-darwin"]).unwrap();
    assert_eq!(
        std::fs::read_to_string(out.path().join("bin/tool")).unwrap(),
        "mac arm64"
    );
    assert!(!out.path().join("bin/x86_64-unknown-linux-gnu").exists());
    assert!(!out.path().join("bin/aarch64-apple-darwin").exists());
    assert!(out.path().join("pages/home.md").exists());
    assert!(out.path().join("extension.toml").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(out.path().join("bin/tool"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "the program is executable");
    }
}

#[test]
fn packing_the_same_files_makes_the_same_bytes() {
    let dir = package();
    assert_eq!(pack(dir.path()).unwrap().1, pack(dir.path()).unwrap().1);
}

#[test]
fn an_unsigned_archive_is_refused() {
    let dir = package();
    let (_, archive) = pack(dir.path()).unwrap();
    let key = SecretKey::generate().unwrap();
    assert_eq!(
        open(&archive, &trusting(&key)).unwrap_err(),
        ArchiveError::Unsigned {
            id: "acme.tool".into(),
            version: "1.2.0".into()
        }
    );
}

#[test]
fn a_tampered_file_is_refused() {
    let key = SecretKey::generate().unwrap();
    let mut files = entries(&signed(&key)).unwrap();
    let (_, bytes) = files.iter_mut().find(|(p, _)| p == "pages/home.md").unwrap();
    bytes[2] ^= 0x01;
    let archive = write_entries(&files).unwrap();
    match open(&archive, &trusting(&key)).unwrap_err() {
        ArchiveError::FileTampered { path, .. } => assert_eq!(path, "pages/home.md"),
        other => panic!("refused, but as {other:?}"),
    }
}

#[test]
fn a_flipped_byte_anywhere_changes_nothing_that_is_accepted() {
    let key = SecretKey::generate().unwrap();
    let trust = trusting(&key);
    let archive = signed(&key);
    let files = entries(&archive).unwrap();
    // Every byte past gzip's header (its date and OS byte say nothing of
    // the contents), one at a time. Most are refused; one that is not
    // decompresses to the very same files (a deflate distance moved
    // within the tar's zero padding).
    let mut refused = 0;
    for at in 10..archive.len() {
        let mut bad = archive.clone();
        bad[at] ^= 0x20;
        match open(&bad, &trust) {
            Err(_) => refused += 1,
            Ok(_) => assert_eq!(entries(&bad).unwrap(), files, "byte {at} changed a file"),
        }
    }
    assert!(refused > (archive.len() - 10) * 9 / 10, "{refused} refused");
}

#[test]
fn a_tampered_index_is_refused() {
    let key = SecretKey::generate().unwrap();
    let mut files = entries(&signed(&key)).unwrap();
    let (_, index) = files.iter_mut().find(|(p, _)| p == INDEX).unwrap();
    let text = String::from_utf8(index.clone())
        .unwrap()
        .replace("1.2.0", "9.9.9");
    *index = text.into_bytes();
    let archive = write_entries(&files).unwrap();
    assert_eq!(
        open(&archive, &trusting(&key)).unwrap_err(),
        ArchiveError::IndexTampered {
            signer: "test key".into()
        }
    );
}

#[test]
fn an_added_file_is_refused() {
    let key = SecretKey::generate().unwrap();
    let mut files = entries(&signed(&key)).unwrap();
    files.push(("bin/x86_64-unknown-linux-gnu/extra".into(), b"!".to_vec()));
    let archive = write_entries(&files).unwrap();
    assert_eq!(
        open(&archive, &trusting(&key)).unwrap_err(),
        ArchiveError::Unlisted {
            path: "bin/x86_64-unknown-linux-gnu/extra".into()
        }
    );
}

#[test]
fn a_wrong_key_is_refused() {
    let signer = SecretKey::generate().unwrap();
    let other = SecretKey::generate().unwrap();
    let archive = signed(&signer);
    match open(&archive, &trusting(&other)).unwrap_err() {
        ArchiveError::UntrustedKey { key_id, trusted } => {
            assert_eq!(key_id, signer.key_id());
            assert_eq!(trusted, [format!("test key ({})", other.key_id())]);
        }
        other => panic!("refused, but as {other:?}"),
    }
    // The org key alone does not trust a throwaway key either.
    assert!(matches!(
        open(&archive, &Trust::org()).unwrap_err(),
        ArchiveError::UntrustedKey { .. }
    ));
}

#[test]
fn a_garbled_signature_is_refused() {
    let key = SecretKey::generate().unwrap();
    let mut files = entries(&signed(&key)).unwrap();
    let (_, sig) = files.iter_mut().find(|(p, _)| p == SIGNATURE).unwrap();
    *sig = b"untrusted comment: x\nnot base64\n".to_vec();
    let archive = write_entries(&files).unwrap();
    assert!(matches!(
        open(&archive, &trusting(&key)).unwrap_err(),
        ArchiveError::BadSignatureFile(_)
    ));
}

#[test]
fn a_path_out_of_the_package_is_refused() {
    // `write_entries` refuses to write one; a hand-made tar is refused on
    // the way in.
    assert!(matches!(
        write_entries(&[("../evil".into(), b"x".to_vec())]).unwrap_err(),
        ArchiveError::Unsafe { .. }
    ));
    let mut raw = Vec::new();
    {
        let gz = flate2::write::GzEncoder::new(&mut raw, flate2::Compression::fast());
        let mut tar = tar::Builder::new(gz);
        let mut header = tar::Header::new_gnu();
        header.set_size(1);
        header.set_entry_type(tar::EntryType::Regular);
        // `set_path` refuses `..`; write the name field directly.
        header.as_old_mut().name[..8].copy_from_slice(b"../evil\0");
        header.set_cksum();
        tar.append(&header, &b"x"[..]).unwrap();
        tar.into_inner().unwrap().finish().unwrap();
    }
    assert!(matches!(
        entries(&raw).unwrap_err(),
        ArchiveError::Unsafe { .. }
    ));
}

#[test]
fn no_build_for_this_platform_is_said() {
    let key = SecretKey::generate().unwrap();
    let v = open(&signed(&key), &trusting(&key)).unwrap();
    let out = tempfile::tempdir().unwrap();
    assert_eq!(
        v.extract_for(out.path(), &["aarch64-unknown-linux-gnu"])
            .unwrap_err(),
        ArchiveError::NoBuild {
            host: "aarch64-unknown-linux-gnu".into(),
            has: vec![
                "aarch64-apple-darwin".into(),
                "x86_64-unknown-linux-gnu".into()
            ],
        }
    );
}

#[test]
fn a_platform_neutral_one_runs_anywhere() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("extension.toml"), LUAU).unwrap();
    std::fs::write(dir.path().join("main.luau"), "return 1").unwrap();
    let key = SecretKey::generate().unwrap();
    let archive = sign_archive(&pack(dir.path()).unwrap().1, &key).unwrap();
    let v = open(&archive, &trusting(&key)).unwrap();
    assert!(v.index.platforms.is_empty());
    assert_eq!(v.cannot_run(), None);
    let out = tempfile::tempdir().unwrap();
    v.extract_for(out.path(), &[]).unwrap();
    assert!(out.path().join("main.luau").exists());
}

#[test]
fn a_process_build_without_its_program_is_not_packed() {
    let dir = package();
    std::fs::remove_file(dir.path().join("bin/aarch64-apple-darwin/tool")).unwrap();
    std::fs::write(dir.path().join("bin/aarch64-apple-darwin/other"), "x").unwrap();
    assert!(matches!(
        pack(dir.path()).unwrap_err(),
        ArchiveError::Manifest(m) if m.contains("aarch64-apple-darwin")
    ));
}

#[test]
fn a_key_survives_its_files() {
    let key = SecretKey::generate().unwrap();
    let again = SecretKey::parse(&key.secret_file()).unwrap();
    assert_eq!(again.public(), key.public());
    assert_eq!(again.key_id(), key.key_id());
    // An encrypted key (kdf `Sc`) is refused with how to make one.
    let text = key.secret_file();
    let line = text.lines().nth(1).unwrap();
    let mut raw = base64::engine::general_purpose::STANDARD
        .decode(line)
        .unwrap();
    raw[2..4].copy_from_slice(b"Sc");
    let enc = base64::engine::general_purpose::STANDARD.encode(raw);
    let err = SecretKey::parse(&format!("untrusted comment: x\n{enc}\n")).unwrap_err();
    assert!(err.to_string().contains("minisign -G -W"), "{err}");
}

#[test]
fn a_registry_index_is_checked() {
    let key = SecretKey::generate().unwrap();
    let archive = signed(&key);
    let sha = {
        use sha2::Digest as _;
        hex::encode(sha2::Sha256::digest(&archive))
    };
    let good = format!(
        r#"name = "Lab"

[[extensions]]
id = "acme.tool"
name = "Tool"
version = "1.2.0"
kind = "process"
archive = "archives/acme.tool-1.2.0.archext"
sha256 = "{sha}"
platforms = ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]
changelog = "First."

[[extensions]]
id = "acme.lane"
version = "0.1.0"
kind = "wasm"
git = "https://example.com/lane"
rev = "v0.1.0"
"#
    );
    let fetch = |at: &str| {
        (at == "archives/acme.tool-1.2.0.archext")
            .then(|| archive.clone())
            .ok_or_else(|| "not here".to_owned())
    };
    let report = registry::check(&good, &fetch, &trusting(&key)).unwrap();
    assert!(report.ok(), "{:?}", report.problems);
    assert_eq!((report.entries, report.archives), (2, 1));

    // Signed by a key the registry does not trust.
    let report = registry::check(&good, &fetch, &Trust::org()).unwrap();
    assert!(report.problems[0].contains("does not trust"), "{report:?}");

    // The pinned hash, a version the archive does not have, an unknown key.
    let bad = good
        .replace(&sha, &"0".repeat(64))
        .replace("rev = \"v0.1.0\"", "rev = \"v0.1.0\"\nhomepage = \"x\"");
    let report = registry::check(&bad, &fetch, &trusting(&key)).unwrap();
    assert_eq!(report.problems.len(), 2, "{:?}", report.problems);
    assert!(report.problems[0].contains("sha256"));
    assert!(report.problems[1].contains("unknown key `homepage`"));
}
