//! The owner's side of a share link's extensions (#710 L2, G1).
//!
//! Choosing, when sharing, which extensions a link's guests run
//! ([`crate::share_needs`]), each from the distribution's linked core or
//! as a signed archive the link carries.

use std::path::Path;

use semver::VersionReq;
use sha2::Digest as _;

use crate::catalog::Origin;
use crate::extensions::Extensions;
use crate::install::Fetched;
use crate::project::Requirement;
use crate::share_needs::{LinkNeeds, LinkPackage, file_path, packages_dir};

/// One extension the owner picks for a link (`EXT_SHARE_NEEDS_SET`).
#[derive(Clone, Debug, PartialEq, Eq, facet::Facet)]
pub struct LinkPick {
    pub id: String,
    /// The link depends on it (when not given); `false`: its guests are
    /// offered it.
    pub required: Option<bool>,
    /// The hosts the owner lets it fetch: each one its manifest asks for.
    pub hosts: Option<Vec<String>>,
}

impl Extensions {
    /// What share link `code` of the vault at `root` runs: none when it
    /// names none.
    ///
    /// # Errors
    ///
    /// Its file is there and does not read.
    pub fn share_needs(&self, root: &Path, code: &str) -> Result<LinkNeeds, String> {
        if !crate::share_needs::is_code(code) {
            return Err(format!("{code:?} is not a share code"));
        }
        let rel = file_path(code);
        // No link followed, and its size before it is read.
        let Some(path) = crate::vault_needs::real(root, &rel, true)? else {
            return Ok(LinkNeeds::default());
        };
        let size = std::fs::symlink_metadata(&path)
            .map_err(|e| format!("{rel}: {e}"))?
            .len();
        if size > u64::try_from(crate::share_needs::MAX_FILE).unwrap_or(u64::MAX) {
            return Err(format!(
                "{rel}: {size} bytes, more than {}",
                crate::share_needs::MAX_FILE
            ));
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{rel}: {e}"))?;
        LinkNeeds::from_toml(&text).map_err(|e| format!("{rel}: {e}"))
    }

    /// Makes `picks` what share link `code` of the vault at `root` runs:
    /// each running here, from the linked core when its running copy is
    /// the built-in, else as the signed archive of the version installed
    /// (fetched from its registry, verified, copied into the link's
    /// packages folder, its SHA-256 pinned). Archives no pick needs any
    /// more are removed. An empty `picks` removes the file.
    ///
    /// # Errors
    ///
    /// A pick is not running here, has no signed archive (a development
    /// copy, a folder install: a guest takes signed packages only), names
    /// a host its manifest does not ask for, or a file cannot be written.
    pub fn set_share_needs(
        &self,
        root: &Path,
        code: &str,
        picks: &[LinkPick],
    ) -> Result<LinkNeeds, String> {
        if !crate::share_needs::is_code(code) {
            return Err(format!("{code:?} is not a share code"));
        }
        let mut needs = LinkNeeds::default();
        let mut archives: Vec<(String, Vec<u8>)> = Vec::new();
        for pick in picks {
            let chosen = self
                .chosen()
                .into_iter()
                .find(|c| c.runs.id == pick.id)
                .ok_or_else(|| format!("{} is not here", pick.id))?;
            let m = &chosen.runs.package.manifest;
            let hosts = pick.hosts.clone().unwrap_or_default();
            if let Some(h) = hosts
                .iter()
                .find(|h| !m.capabilities.hosts.iter().any(|a| a == *h))
            {
                return Err(format!("{}: it does not ask for host {h}", pick.id));
            }
            let requirement = Requirement::new(&m.id)
                .named(&m.name)
                .versions(format!("^{}", m.version));
            let requirement = if pick.required.unwrap_or(true) {
                requirement
            } else {
                requirement.optional()
            };
            match chosen.runs.origin {
                // From the guest's linked core: no package.
                Origin::Builtin => {}
                Origin::Installed => {
                    let exact =
                        VersionReq::parse(&format!("={}", m.version)).map_err(|e| e.to_string())?;
                    let bytes = self.signed_archive(&m.id, &exact)?;
                    let file = format!("{}-{}.archext", m.id, m.version);
                    needs.packages.push(LinkPackage {
                        id: m.id.clone(),
                        version: m.version.to_string(),
                        archive: file.clone(),
                        sha256: hex::encode(sha2::Sha256::digest(&bytes)),
                    });
                    archives.push((file, bytes));
                }
                Origin::Dev => {
                    return Err(format!(
                        "{} is a development copy: a link's guests take signed packages only",
                        pick.id
                    ));
                }
            }
            if !hosts.is_empty() {
                needs.hosts.insert(m.id.clone(), hosts);
            }
            needs.requirements.extensions.push(requirement);
        }
        needs.check()?;
        let rel = file_path(code);
        let dir_rel = packages_dir(code);
        if needs.requirements.extensions.is_empty() {
            // Removed only as what they are: never through a link.
            if let Some(file) = crate::vault_needs::real(root, &rel, true)? {
                std::fs::remove_file(&file).map_err(|e| format!("{rel}: {e}"))?;
            }
            if let Some(dir) = crate::vault_needs::real(root, &dir_rel, false)? {
                std::fs::remove_dir_all(&dir).map_err(|e| format!("{dir_rel}: {e}"))?;
            }
            return Ok(needs);
        }
        for (name, bytes) in &archives {
            crate::vault_needs::real_write(root, &format!("{dir_rel}/{name}"), bytes)?;
        }
        // What no pick needs any more goes: each a plain file in a real
        // folder, a link only unlinked (never what it points at).
        if let Some(dir) = crate::vault_needs::real(root, &dir_rel, false)? {
            let entries = std::fs::read_dir(&dir).map_err(|e| format!("{dir_rel}: {e}"))?;
            for e in entries.filter_map(Result::ok) {
                let name = e.file_name().to_string_lossy().into_owned();
                if !archives.iter().any(|(n, _)| *n == name) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        crate::vault_needs::real_write(root, &rel, needs.to_toml()?.as_bytes())?;
        Ok(needs)
    }

    /// The signed archive of `id` at `req` from a registry, verified.
    fn signed_archive(&self, id: &str, req: &VersionReq) -> Result<Vec<u8>, String> {
        let (source, listing) = self.find(id, req, &[]).map_err(|e| e.to_string())?;
        if !matches!(listing.location, crate::index::Location::Archive { .. }) {
            return Err(format!(
                "{id} {}: its registry has no signed archive of it; a link's guests take signed packages only",
                listing.version
            ));
        }
        let tmp = std::env::temp_dir().join(format!(
            "architect-share-pick-{}-{}",
            std::process::id(),
            id
        ));
        let fetched = source.fetch(&listing, &tmp);
        let _ = std::fs::remove_dir_all(&tmp);
        match fetched.map_err(|e| e.to_string())? {
            Fetched::Archive(bytes) => {
                self.installs()
                    .open_archive(&bytes, source.as_ref())
                    .map_err(|e| e.to_string())?;
                Ok(bytes)
            }
            _ => Err(format!("{id}: its registry did not hand over an archive")),
        }
    }
}
