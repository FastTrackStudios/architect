//! A person's installed extensions on all their devices (#683's decision,
//! #710 L2): what they trusted and installed on one device installs on
//! their others with the same grants and no new prompt.
//!
//! The record syncs with the vault, in its own folder:
//!
//! ```toml
//! # .architect/people/<person>/extensions.toml
//! [[installs]]
//! id = "acme.greeter"
//! version = "0.2.0"
//! pin = "~0.2"                       # when pinned
//! signer = "Acme"                    # who signed it, as installed
//! signer_key = "RWQ…"                # that key
//! archive = "acme.greeter-0.2.0.archext"   # in .architect/extensions/
//! sha256 = "…"
//!
//! [[installs.grants]]                # the person's grants, as given
//! scope = "device"
//! capabilities = { reads = ["selection"], ui = ["notify"] }
//! ```
//!
//! [`crate::extensions::Extensions::publish_installs`] writes it (and
//! copies each signed archive into the vault's carried folder);
//! [`crate::extensions::Extensions::apply_installs`] installs from it on
//! another device of the same person. The same trust rules hold there as
//! for any archive (#733's review): the archive is the one the record pins
//! (SHA-256), at most 16 MB, no link followed, signed, and by **a key that
//! device already trusts** (the org key, or a publisher of its own
//! registries) that is also the key the record names. A vault, which other
//! people can write, cannot bring a new key. No downgrade, no step past an
//! installed copy's pin; an installed copy signed by another key is kept
//! (`SignerChanged`).
//!
//! Other people (the vault's other members) are not in it: a record is
//! the person's, grants never transfer, and they get the vault's needs list
//! and one prompt.

use serde::{Deserialize, Serialize};

use crate::consent::Scope;
use crate::manifest::Capabilities;

/// The most a person's record may be.
pub const MAX_RECORD: u64 = 256 * 1024;

/// Whether `person` can name a record: an account id of letters, digits,
/// `-` and `_`, 1 to 64 of them (so it stays one folder).
#[must_use]
pub fn is_person(person: &str) -> bool {
    !person.is_empty()
        && person.len() <= 64
        && person
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Where `person`'s record is, from the vault's root.
#[must_use]
pub fn record_path(person: &str) -> String {
    format!(".architect/people/{person}/extensions.toml")
}

/// One grant of a person's, as given on the device that installed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedGrant {
    pub scope: Scope,
    pub capabilities: Capabilities,
}

/// One extension a person installed, as their other devices take it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedInstall {
    pub id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    /// The name of the key it was signed by, as installed.
    #[serde(default)]
    pub signer: String,
    /// That key (a minisign `.pub`'s base64 line).
    pub signer_key: String,
    /// Its signed archive's file name in `.architect/extensions/`.
    pub archive: String,
    /// That archive's SHA-256, lower-case hex.
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<SyncedGrant>,
}

/// A person's installed extensions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonInstalls {
    #[serde(default)]
    pub installs: Vec<SyncedInstall>,
}

impl PersonInstalls {
    /// From the record's text.
    ///
    /// # Errors
    ///
    /// Not TOML, or an archive name that is not a plain `.archext` name.
    pub fn from_toml(text: &str) -> Result<Self, String> {
        let me: Self = toml::from_str(text).map_err(|e| e.to_string())?;
        if let Some(bad) = me.installs.iter().find(|i| !plain_archive(&i.archive)) {
            return Err(format!(
                "{}: {:?} is not a .archext file name",
                bad.id, bad.archive
            ));
        }
        Ok(me)
    }

    /// As the record's text.
    ///
    /// # Errors
    ///
    /// The encoder's message.
    pub fn to_toml(&self) -> Result<String, String> {
        toml::to_string(self).map_err(|e| e.to_string())
    }
}

fn plain_archive(name: &str) -> bool {
    name.ends_with(".archext")
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && !name.contains("..")
}

#[cfg(feature = "host")]
mod apply {
    use std::path::Path;

    use semver::Version;
    use sha2::Digest as _;

    use super::{MAX_RECORD, PersonInstalls, SyncedGrant, SyncedInstall, is_person, record_path};
    use crate::extensions::Extensions;
    use crate::index::{Listing, Location};
    use crate::install::{InstallError, Trust};
    use crate::vault_needs::{CARRIED, MAX_ARCHIVE, MAX_PEEK, real};

    /// What applying one synced install did.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub enum Applied {
        /// Installed here at the record's version, its grants given.
        Installed { id: String, version: String },
        /// Here already at that version.
        Here { id: String },
    }

    impl Extensions {
        /// Writes `person`'s record in the vault at `root`: each extension
        /// installed here from a signed archive, with its pin, signer and
        /// the person's grants, and its archive copied into the vault's
        /// carried folder. What their other devices take
        /// ([`Self::apply_installs`]).
        ///
        /// # Errors
        ///
        /// `person` is not an account id, `.architect` is a link, or a file
        /// cannot be written.
        pub fn publish_installs(
            &self,
            root: &Path,
            person: &str,
        ) -> Result<PersonInstalls, String> {
            if !is_person(person) {
                return Err(format!("{person:?} is not an account id"));
            }
            for dir in [".architect", CARRIED] {
                if real(root, dir, false)?.is_none() {
                    std::fs::create_dir_all(root.join(dir)).map_err(|e| format!("{dir}: {e}"))?;
                }
            }
            let grants = self.host().grants();
            let mut record = PersonInstalls::default();
            for install in self
                .installs()
                .list()
                .into_iter()
                .filter(crate::install::Install::is_extension)
            {
                let (Some(key), Some(bytes)) = (
                    install.signer_key.clone(),
                    self.installs().archive_of(&install.id),
                ) else {
                    continue;
                };
                let archive = format!("{}-{}.archext", install.id, install.version);
                let at = root.join(CARRIED).join(&archive);
                std::fs::write(&at, &bytes).map_err(|e| format!("{}: {e}", at.display()))?;
                let signer = self
                    .installs()
                    .trust_with(&self.sources())
                    .keys()
                    .iter()
                    .find(|k| k.key.trim() == key.trim())
                    .map(|k| k.name.clone())
                    .unwrap_or_default();
                record.installs.push(SyncedInstall {
                    id: install.id.clone(),
                    version: install.version.to_string(),
                    pin: install.pin.clone(),
                    signer,
                    signer_key: key,
                    archive,
                    sha256: hex::encode(sha2::Sha256::digest(&bytes)),
                    grants: grants
                        .as_ref()
                        .map(|g| {
                            g.list()
                                .into_iter()
                                .filter(|g| g.id == install.id)
                                .map(|g| SyncedGrant {
                                    scope: g.scope,
                                    capabilities: g.capabilities,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                });
            }
            let rel = record_path(person);
            let path = root.join(&rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("{}: {e}", parent.display()))?;
            }
            if real(root, &rel, true).is_err() {
                return Err(format!("{rel} is not a plain file"));
            }
            std::fs::write(&path, record.to_toml()?).map_err(|e| format!("{rel}: {e}"))?;
            Ok(record)
        }

        /// `person`'s record in the vault at `root`: none when there is none.
        ///
        /// # Errors
        ///
        /// It, or a folder on its way, is a link; it is larger than
        /// [`MAX_RECORD`]; it does not read.
        pub fn person_installs(&self, root: &Path, person: &str) -> Result<PersonInstalls, String> {
            if !is_person(person) {
                return Err(format!("{person:?} is not an account id"));
            }
            let rel = record_path(person);
            let Some(path) = real(root, &rel, true)? else {
                return Ok(PersonInstalls::default());
            };
            let size = std::fs::symlink_metadata(&path)
                .map_err(|e| e.to_string())?
                .len();
            if size > MAX_RECORD {
                return Err(format!("{rel}: {size} bytes, more than {MAX_RECORD}"));
            }
            PersonInstalls::from_toml(&std::fs::read_to_string(&path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("{rel}: {e}"))
        }

        /// Installs here what `person` installed on another device (their
        /// record in the vault at `root`), with their grants and no
        /// prompt. Each is checked as any archive is: the one the record
        /// pins, at most 16 MB, no link, signed by the key the record names,
        /// which this device must already trust (the org key, or a publisher
        /// of its registries); never a downgrade or past a pin; an installed
        /// copy signed by another key is kept. Each outcome, or why not.
        ///
        /// # Errors
        ///
        /// The record does not read.
        pub fn apply_installs(
            &self,
            root: &Path,
            person: &str,
        ) -> Result<Vec<Result<Applied, (String, InstallError)>>, String> {
            let record = self.person_installs(root, person)?;
            let out = record
                .installs
                .iter()
                .map(|s| self.apply_one(root, s).map_err(|e| (s.id.clone(), e)))
                .collect();
            let _ = self.sync();
            Ok(out)
        }

        fn apply_one(&self, root: &Path, s: &SyncedInstall) -> Result<Applied, InstallError> {
            let version =
                Version::parse(&s.version).map_err(|e| InstallError::Mismatch(e.to_string()))?;
            if self
                .installs()
                .get(&s.id)
                .is_some_and(|i| i.version == version)
            {
                return Ok(Applied::Here { id: s.id.clone() });
            }
            // The record names the key; this device must already trust it.
            let here = self.installs().trust_with(&self.sources());
            let Some(known) = here
                .keys()
                .iter()
                .find(|k| k.key.trim() == s.signer_key.trim())
            else {
                return Err(InstallError::NotAllowed(format!(
                    "{}: signed by {}, a key this device does not trust: add its registry here",
                    s.id,
                    if s.signer.is_empty() {
                        "a key"
                    } else {
                        s.signer.as_str()
                    }
                )));
            };
            let trust = Trust::none().with(known.name.clone(), known.key.clone());
            let rel = format!("{CARRIED}/{}", s.archive);
            let path = real(root, &rel, true)
                .map_err(InstallError::Io)?
                .ok_or_else(|| InstallError::NotFound(format!("{rel} in the vault")))?;
            let size = std::fs::symlink_metadata(&path)
                .map_err(|e| InstallError::Io(e.to_string()))?
                .len();
            if size > MAX_ARCHIVE {
                return Err(InstallError::Io(format!(
                    "{rel}: {size} bytes, more than {MAX_ARCHIVE}"
                )));
            }
            let bytes = std::fs::read(&path).map_err(|e| InstallError::Io(e.to_string()))?;
            let got = hex::encode(sha2::Sha256::digest(&bytes));
            if got != s.sha256.to_ascii_lowercase() {
                return Err(InstallError::Mismatch(format!(
                    "{rel}: its sha256 is {got}, the record pinned {}",
                    s.sha256
                )));
            }
            let p = architect_extension_archive::peek(&bytes, &trust, MAX_PEEK)?;
            let manifest = p
                .manifest
                .as_deref()
                .map(crate::manifest::Manifest::parse)
                .transpose()
                .map_err(|e| InstallError::Mismatch(e.to_string()))?
                .ok_or_else(|| InstallError::Mismatch("no extension.toml".to_owned()))?;
            let listing = Listing {
                id: s.id.clone(),
                name: manifest.name.clone(),
                version,
                kind: p.index.kind.clone(),
                description: manifest.description,
                category: String::new(),
                location: Location::Archive {
                    url: rel,
                    sha256: Some(s.sha256.clone()),
                },
                platforms: p.index.platforms,
                changelog: String::new(),
            };
            self.no_downgrade(&listing)?;
            // The person's grants first, so it loads with them.
            if let Some(grants) = self.host().grants() {
                for g in &s.grants {
                    grants.grant(
                        &s.id,
                        &s.version,
                        g.capabilities.clone(),
                        g.scope.clone(),
                        "sync",
                    );
                }
            }
            let install =
                self.installs()
                    .install_bytes(bytes, &listing, "this person's devices", trust)?;
            if let Some(pin) = &s.pin {
                self.installs().set_pin(&install.id, Some(pin))?;
            }
            Ok(Applied::Installed {
                id: install.id,
                version: install.version.to_string(),
            })
        }
    }
}

#[cfg(feature = "host")]
pub use apply::Applied;
