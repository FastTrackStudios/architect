//! `archext install`: a signed extension archive, checked and unpacked,
//! for a build that installs extensions with no app running: Architect's
//! Nix function `lib.architectExtension` (`flake.nix`), which the node's
//! NixOS module installs from (#583).
//!
//! ```text
//! archext install <archive> --id <id> --version <version> --sha256 <hex>
//!                 [--org] [--key <name>=<minisign public key>]... <out>
//! ```
//!
//! It checks, in order, and refuses on the first that fails:
//! - the archive's SHA-256 is the pinned one;
//! - its signature, index and files, with [`architect_extension_archive::open`]
//!   (the check every app runs on an install), against the keys given:
//!   `--org` for the `FastTrackStudios` release key, each `--key` for a
//!   publisher's;
//! - it is the pinned id and version;
//!
//! then writes its files for this platform into `<out>`
//! ([`architect_extension_archive::Verified::extract`]). Nothing is
//! written before every check passes.

use std::path::PathBuf;
use std::process::ExitCode;

use architect_extension_archive::{Trust, open};
use sha2::Digest as _;

const USAGE: &str = "usage: archext install <archive> --id <id> --version <version> --sha256 <hex> [--org] [--key <name>=<public key>]... <out>";

struct Install {
    archive: PathBuf,
    id: String,
    version: String,
    sha256: String,
    trust: Trust,
    out: PathBuf,
}

fn parse(args: &[String]) -> Result<Install, String> {
    let mut it = args.iter();
    if it.next().map(String::as_str) != Some("install") {
        return Err(USAGE.to_owned());
    }
    let (mut id, mut version, mut sha256) = (None, None, None);
    let mut trust = Trust::none();
    let mut paths = Vec::new();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--id" => id = Some(value("--id")?),
            "--version" => version = Some(value("--version")?),
            "--sha256" => sha256 = Some(value("--sha256")?.to_ascii_lowercase()),
            "--org" => trust = trust.and(&Trust::org()),
            "--key" => {
                let kv = value("--key")?;
                let (name, key) = kv
                    .split_once('=')
                    .ok_or_else(|| format!("--key {kv}: <name>=<public key>"))?;
                trust = trust.with(name, key);
            }
            other if other.starts_with("--") => return Err(format!("{other}?\n{USAGE}")),
            path => paths.push(PathBuf::from(path)),
        }
    }
    let [archive, out]: [PathBuf; 2] = paths
        .try_into()
        .map_err(|_| format!("an archive and an out folder\n{USAGE}"))?;
    if trust.keys().is_empty() {
        return Err("no key to trust: --org, or a --key".to_owned());
    }
    Ok(Install {
        archive,
        id: id.ok_or_else(|| format!("--id\n{USAGE}"))?,
        version: version.ok_or_else(|| format!("--version\n{USAGE}"))?,
        sha256: sha256.ok_or_else(|| format!("--sha256\n{USAGE}"))?,
        trust,
        out,
    })
}

fn install(job: &Install) -> Result<String, String> {
    let bytes =
        std::fs::read(&job.archive).map_err(|e| format!("{}: {e}", job.archive.display()))?;
    let got = hex::encode(sha2::Sha256::digest(&bytes));
    if got != job.sha256 {
        return Err(format!(
            "{}: its sha256 is {got}, pinned {}",
            job.archive.display(),
            job.sha256
        ));
    }
    let verified = open(&bytes, &job.trust).map_err(|e| e.to_string())?;
    let index = &verified.index;
    if index.id != job.id || index.version != job.version {
        return Err(format!(
            "it is {} {}, pinned {} {}",
            index.id, index.version, job.id, job.version
        ));
    }
    if job.out.exists() {
        return Err(format!("{} exists: not writing over it", job.out.display()));
    }
    verified.extract(&job.out).map_err(|e| e.to_string())?;
    Ok(format!(
        "{} {}: signed by {}, unpacked in {}",
        index.id,
        index.version,
        verified.signer,
        job.out.display()
    ))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args).and_then(|job| install(&job)) {
        Ok(said) => {
            println!("{said}");
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("archext: {why}");
            ExitCode::FAILURE
        }
    }
}
