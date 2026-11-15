//! A registry compiled into the app (`BundledSource`).
//!
//! Packages whose files the build carries, offered like any registry's
//! (searched, previewed, installed through the consent prompt), with no
//! network and no folder to add.
//!
//! What a distribution uses for the extensions it ships to try without
//! linking them in: the showcase's (#710), which the showcase vault names
//! and offers with "Install all". Nothing of one runs before a person says
//! yes; installed, it is a copy like any other, removed, pinned and
//! updated the same way. Being the app's own build is what vouches for
//! it, as a folder registry vouches for its path packages.

use std::path::Path;

use crate::index::{Index, Listing, Location};
use crate::install::{Fetched, InstallError, Source};
use crate::manifest::{MANIFEST, Manifest};

/// One package the build carries: its folder's name and every file in
/// it, by path from the folder.
#[derive(Clone, Debug)]
pub struct BundledPackage {
    pub folder: String,
    pub files: Vec<(String, &'static [u8])>,
}

/// A registry of packages compiled into the app.
#[derive(Clone, Debug)]
pub struct BundledSource {
    name: String,
    packages: Vec<BundledPackage>,
}

impl BundledSource {
    /// The registry `name` of `packages`.
    #[must_use]
    pub fn new(name: impl Into<String>, packages: Vec<BundledPackage>) -> Self {
        Self {
            name: name.into(),
            packages,
        }
    }

    fn manifest_of(p: &BundledPackage) -> Option<Manifest> {
        let (_, bytes) = p.files.iter().find(|(path, _)| path == MANIFEST)?;
        Manifest::parse(std::str::from_utf8(bytes).ok()?).ok()
    }
}

impl Source for BundledSource {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn refresh(&self) -> Result<(), InstallError> {
        Ok(())
    }

    fn index(&self) -> Result<Index, InstallError> {
        Ok(Index {
            name: self.name.clone(),
            listings: self
                .packages
                .iter()
                .filter_map(|p| {
                    let m = Self::manifest_of(p)?;
                    Some(Listing {
                        id: m.id.clone(),
                        name: m.name.clone(),
                        version: m.version.clone(),
                        kind: m.kind.name().to_owned(),
                        description: m.description,
                        category: String::new(),
                        location: Location::Path(p.folder.clone()),
                        platforms: Vec::new(),
                        changelog: String::new(),
                    })
                })
                .collect(),
        })
    }

    fn fetch(&self, listing: &Listing, _: &Path) -> Result<Fetched, InstallError> {
        self.fetch_files(listing)
    }

    /// Its files as the build carries them: [`crate::install::Installs`]
    /// writes them (checking each stays inside the package) wherever it
    /// keeps installs, a folder or a browser's storage.
    fn fetch_files(&self, listing: &Listing) -> Result<Fetched, InstallError> {
        let Location::Path(folder) = &listing.location else {
            return Err(InstallError::NotFound(listing.id.clone()));
        };
        let package = self
            .packages
            .iter()
            .find(|p| &p.folder == folder)
            .ok_or_else(|| InstallError::NotFound(format!("{} in {}", listing.id, self.name)))?;
        Ok(Fetched::Files(
            package
                .files
                .iter()
                .map(|(path, bytes)| (path.clone(), bytes.to_vec()))
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{BundledPackage, BundledSource};
    use crate::index::Location;
    use crate::install::{Fetched, Source};

    const MANIFEST: &[u8] =
        b"id = \"x.hello\"\nname = \"Hello\"\nversion = \"0.2.0\"\nkind = \"data\"\n";

    #[test]
    fn a_bundled_package_is_listed_and_handed_over_as_files() {
        let source = BundledSource::new(
            "Built in",
            vec![BundledPackage {
                folder: "hello".into(),
                files: vec![
                    ("extension.toml".into(), MANIFEST),
                    ("themes/a.json".into(), b"{}"),
                ],
            }],
        );
        let index = source.index().unwrap();
        assert_eq!(index.listings.len(), 1);
        let l = &index.listings[0];
        assert_eq!(
            (l.id.as_str(), l.version.to_string()),
            ("x.hello", "0.2.0".to_owned())
        );
        assert_eq!(l.location, Location::Path("hello".into()));
        let tmp = tempfile::tempdir().unwrap();
        let Fetched::Files(files) = source.fetch(l, tmp.path()).unwrap() else {
            panic!("handed over as files");
        };
        assert!(
            files
                .iter()
                .any(|(p, b)| p == "themes/a.json" && b == b"{}")
        );
    }

    #[test]
    fn a_path_leaving_the_package_is_refused() {
        let source = BundledSource::new(
            "Built in",
            vec![BundledPackage {
                folder: "bad".into(),
                files: vec![("extension.toml".into(), MANIFEST), ("../x".into(), b"no")],
            }],
        );
        let l = source.index().unwrap().listings.remove(0);
        let tmp = tempfile::tempdir().unwrap();
        let installs = crate::install::Installs::new(tmp.path().join("in"));
        assert!(installs.install(&source, &l).is_err());
        assert!(!tmp.path().join("x").exists());
        // In a browser's storage too.
        let disk = std::sync::Arc::new(crate::disk::MemDisk::new());
        let installs = crate::install::Installs::new("/in").with_disk(disk.clone());
        assert!(installs.install(&source, &l).is_err());
        assert!(
            disk.files().keys().all(|k| k.starts_with("in/")),
            "{:?}",
            disk.files().keys()
        );
    }
}
