//! `cargo xtask ext-*` — extension archives (`.archext`), the same commands
//! for the release workflow (`.github/workflows/extension-release.yml`)
//! and for a person:
//!
//! - `ext-pack <dir>`: the unsigned archive of the package in `<dir>`;
//! - `ext-sign <archive>`: signs it with a minisign secret key (a file, or
//!   the `EXT_SIGNING_KEY` environment variable's text); `--expect` refuses
//!   a key that is not the one named;
//! - `ext-verify <archive>`: opens it as an app would (the org key, and
//!   `--key`s);
//! - `ext-keygen <dir>`: a throwaway key pair, for trials and tests;
//! - `ext-index <registry>`: checks a registry's `index.toml` (schema, each
//!   archive's hash and signature).
//!
//! The secret key's text is never printed; only its key id is.

use std::path::{Path, PathBuf};
use std::process::Command;

use architect_extension_archive::sign::{SecretKey, sign_archive};
use architect_extension_archive::{EXTENSION, Trust, open, pack, registry};

/// The environment variable `ext-sign` reads a key's text from when no
/// `--key` file is given.
pub const KEY_ENV: &str = "EXT_SIGNING_KEY";

/// `ext-pack`: writes the archive, and prints its path.
pub fn pack_cli(dir: &Path, out: Option<&Path>) -> Result<(), String> {
    let (index, bytes) = pack(dir).map_err(|e| e.to_string())?;
    let out = out.map_or_else(
        || PathBuf::from(format!("{}-{}.{EXTENSION}", index.id, index.version)),
        Path::to_path_buf,
    );
    std::fs::write(&out, bytes).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "packed {} {} ({} files{}) -> {}",
        index.id,
        index.version,
        index.files.len(),
        if index.platforms.is_empty() {
            String::new()
        } else {
            format!("; builds for {}", index.platforms.join(", "))
        },
        out.display()
    );
    println!(
        "unsigned: sign it with `cargo xtask ext-sign {}`",
        out.display()
    );
    Ok(())
}

/// A trusted key from the command line: a `.pub` file's path, or its
/// base64 line.
fn read_public(arg: &str) -> Result<String, String> {
    let path = Path::new(arg);
    if path.is_file() {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{arg}: {e}"))?;
        return text
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("untrusted comment:"))
            .map(str::to_owned)
            .ok_or_else(|| format!("{arg}: no key in it"));
    }
    Ok(arg.trim().to_owned())
}

fn trust(keys: &[String], no_org: bool) -> Result<Trust, String> {
    let mut trust = if no_org { Trust::none() } else { Trust::org() };
    for k in keys {
        trust = trust.with(format!("--key {k}"), read_public(k)?);
    }
    Ok(trust)
}

/// `ext-sign`: signs `archive` (in place, or into `out`).
pub fn sign_cli(
    archive: &Path,
    key: Option<&Path>,
    expect: Option<&str>,
    out: Option<&Path>,
) -> Result<(), String> {
    let text = match key {
        Some(path) => {
            std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?
        }
        None => std::env::var(KEY_ENV)
            .ok()
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| format!("no key: give --key <file>, or the key's text in {KEY_ENV}"))?,
    };
    let key = SecretKey::parse(&text).map_err(|e| e.to_string())?;
    if let Some(expect) = expect {
        let want = read_public(expect)?;
        if key.public() != want {
            return Err(format!(
                "the signing key is {}, not the expected key: refusing to sign with it",
                key.key_id()
            ));
        }
    }
    let bytes = std::fs::read(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let signed = sign_archive(&bytes, &key).map_err(|e| e.to_string())?;
    // Signed, then checked as an app would: what is written opens.
    let v = open(
        &signed,
        &Trust::none().with("the signing key", key.public()),
    )
    .map_err(|e| format!("the signed archive does not open: {e}"))?;
    let out = out.unwrap_or(archive);
    std::fs::write(out, signed).map_err(|e| format!("{}: {e}", out.display()))?;
    println!(
        "signed {} {} with key {} -> {}",
        v.index.id,
        v.index.version,
        key.key_id(),
        out.display()
    );
    Ok(())
}

/// `ext-verify`: opens `archive` as an app trusting the org key (unless
/// `no_org`) and `keys` would.
pub fn verify_cli(archive: &Path, keys: &[String], no_org: bool) -> Result<(), String> {
    let bytes = std::fs::read(archive).map_err(|e| format!("{}: {e}", archive.display()))?;
    let v = open(&bytes, &trust(keys, no_org)?).map_err(|e| e.to_string())?;
    println!(
        "{} {} ({}): signed by {}; {} files; {}",
        v.index.id,
        v.index.version,
        v.index.kind,
        v.signer,
        v.index.files.len(),
        if v.index.platforms.is_empty() {
            "platform-neutral".to_owned()
        } else {
            format!("builds for {}", v.index.platforms.join(", "))
        }
    );
    if let Some(why) = v.cannot_run() {
        println!("on this machine: {why}");
    }
    Ok(())
}

/// `ext-keygen`: a throwaway key pair, `<name>.key` (owner-only) and
/// `<name>.pub`, in `dir`.
pub fn keygen_cli(dir: &Path, name: &str) -> Result<(), String> {
    let key = SecretKey::generate().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let secret = dir.join(format!("{name}.key"));
    let public = dir.join(format!("{name}.pub"));
    if secret.exists() {
        return Err(format!(
            "{} exists: not overwriting a key",
            secret.display()
        ));
    }
    std::fs::write(&secret, key.secret_file()).map_err(|e| format!("{}: {e}", secret.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("{}: {e}", secret.display()))?;
    }
    std::fs::write(&public, key.public_file()).map_err(|e| format!("{}: {e}", public.display()))?;
    println!(
        "throwaway key {}: {} (secret, keep it out of git) and {}\npublic: {}",
        key.key_id(),
        secret.display(),
        public.display(),
        key.public()
    );
    Ok(())
}

/// Fetches an index entry's `archive`: a path relative to the registry,
/// `file://`, or an `http(s)` URL (with `curl`).
fn fetch(root: &Path, at: &str) -> Result<Vec<u8>, String> {
    if at.starts_with("https://") || at.starts_with("http://") {
        let out = Command::new("curl")
            .args(["-fsSL", "--max-time", "300", at])
            .output()
            .map_err(|e| format!("cannot run curl: {e}"))?;
        if !out.status.success() {
            return Err(String::from_utf8_lossy(&out.stderr).trim().to_owned());
        }
        return Ok(out.stdout);
    }
    let path = at
        .strip_prefix("file://")
        .map_or_else(|| root.join(at), PathBuf::from);
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

/// `ext-index`: checks `<root>/index.toml`.
pub fn index_cli(root: &Path, keys: &[String], no_org: bool) -> Result<(), String> {
    let path = root.join("index.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let report = registry::check(&text, &|at| fetch(root, at), &trust(keys, no_org)?)?;
    for p in &report.problems {
        eprintln!("  {p}");
    }
    if report.ok() {
        println!(
            "{}: {} entries, {} archives verified",
            path.display(),
            report.entries,
            report.archives
        );
        Ok(())
    } else {
        Err(format!(
            "{}: {} problem(s) in {} entries",
            path.display(),
            report.problems.len(),
            report.entries
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{index_cli, read_public};
    use std::path::Path;

    /// The example registry checks with its own throwaway key, and is
    /// refused by the org key alone.
    #[test]
    fn the_example_registry_checks() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../examples/registry");
        let key = root.join("keys/throwaway.pub");
        let key = key.display().to_string();
        assert!(read_public(&key).unwrap().starts_with("RW"));
        index_cli(&root, &[key], true).unwrap();
        let err = index_cli(&root, &[], false).unwrap_err();
        assert!(err.contains("problem"), "{err}");
    }
}
