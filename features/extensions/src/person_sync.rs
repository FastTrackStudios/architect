//! A person's installed extensions on all their devices (#683's decision,
//! #710 L2).
//!
//! What they trusted and installed on one device installs on their others
//! with the same grants and no new prompt; what they removed or revoked
//! goes there too. The record syncs with the vault, in its own folder, and
//! is **signed by the device that wrote it**:
//!
//! ```toml
//! # .architect/people/<person>/extensions.toml (+ .minisig beside it)
//! person = "alice"
//! device = { name = "Alice's laptop", key = "RWQ…" }
//!
//! [[installs]]
//! id = "acme.greeter"
//! version = "0.2.0"
//! pin = "~0.2"
//! signer = "Acme"
//! signer_key = "RWQ…"
//! archive = "acme.greeter-0.2.0.archext"   # in .architect/extensions/
//! sha256 = "…"
//! at_ms = 1763150000000                    # when, on the device that said so
//! removed = false                          # true: removed there
//! grants = [{ scope = "device", capabilities = { reads = ["selection"] } }]
//! ```
//!
//! A device applies a record only when it is signed by **its own key or
//! the key of one of the person's devices it was told to trust** (once,
//! by the person: [`DeviceKeys::trust`]). A record anyone else wrote, or
//! one not signed, is refused and nothing of it applies: a vault, which
//! others can write, is no way to put code or grants on someone's device.
//! A node also refuses anyone but the person a write under
//! `.architect/people/<person>/` (`vault_live::ExtensionPaths`).
//!
//! Each entry is the last word on its id ([`SyncedInstall::at_ms`]): an
//! install, a removal or a revoke on one device is applied on the others
//! once, never over a later word of their own. The same rules as any
//! archive's hold (#733's review): the archive the entry pins (SHA-256),
//! at most 16 MB, no link followed, signed by the key the person's own
//! device named; no downgrade, no step past a pin; grants written only
//! once it installed.
//!
//! Other people are not in it: grants never transfer between people; they
//! get the vault's needs list and one prompt.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use architect_extension_archive::sign::SecretKey;
use architect_extension_archive::{Trust, verify_detached};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::Digest as _;

use crate::consent::Scope;
use crate::extensions::Extensions;
use crate::index::{Listing, Location};
use crate::install::InstallError;
use crate::manifest::Capabilities;
use crate::vault_needs::{CARRIED, MAX_ARCHIVE, MAX_PEEK, real, real_write};

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

/// Its signature, beside it.
#[must_use]
pub fn signature_path(person: &str) -> String {
    format!("{}.minisig", record_path(person))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// One grant of a person's, as given on the device that installed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedGrant {
    pub scope: Scope,
    pub capabilities: Capabilities,
}

/// The last word on one extension of a person's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncedInstall {
    pub id: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin: Option<String>,
    /// The name of the key its archive was signed by, as installed.
    #[serde(default)]
    pub signer: String,
    /// That key (a minisign `.pub`'s base64 line).
    #[serde(default)]
    pub signer_key: String,
    /// Its signed archive's file name in `.architect/extensions/`.
    #[serde(default)]
    pub archive: String,
    /// That archive's SHA-256, lower-case hex.
    #[serde(default)]
    pub sha256: String,
    /// When this was said, on the device that said it (ms since the
    /// epoch): the later word wins.
    pub at_ms: u64,
    /// Removed on that device: removed on the others.
    #[serde(default)]
    pub removed: bool,
    /// Its grants, all of them (none: revoked).
    #[serde(default)]
    pub grants: Vec<SyncedGrant>,
}

/// A device, as a record names it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceName {
    pub name: String,
    /// Its public key (a minisign `.pub`'s base64 line).
    pub key: String,
}

/// A person's installed extensions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonInstalls {
    /// Whose: the record's folder must be this person's.
    #[serde(default)]
    pub person: String,
    /// The device that wrote it last, and signed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device: Option<DeviceName>,
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
        if let Some(bad) = me
            .installs
            .iter()
            .find(|i| !i.removed && !plain_archive(&i.archive))
        {
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

    fn entry(&self, id: &str) -> Option<&SyncedInstall> {
        self.installs.iter().find(|i| i.id == id)
    }

    fn put(&mut self, entry: SyncedInstall) {
        self.installs.retain(|i| i.id != entry.id);
        self.installs.push(entry);
        self.installs.sort_by(|a, b| a.id.cmp(&b.id));
    }
}

fn plain_archive(name: &str) -> bool {
    name.ends_with(".archext")
        && !name.starts_with('.')
        && !name.contains(['/', '\\', '\0'])
        && !name.contains("..")
}

/// Why a person's record was not applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// It is signed by a key this device does not trust: not one of this
    /// person's devices, or one not trusted here yet ([`DeviceKeys::trust`]
    /// is the person's to say). Nothing of it was applied.
    UntrustedDevice { device: Option<DeviceName> },
    /// It has no signature, or its signature is not over it.
    Unsigned(String),
    /// It is another person's, a link, too large, or does not read.
    Bad(String),
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UntrustedDevice { device } => write!(
                f,
                "written by {}, which this device does not trust as one of yours",
                device.as_ref().map_or_else(
                    || "an unknown device".to_owned(),
                    |d| format!("\"{}\"", d.name)
                )
            ),
            Self::Unsigned(why) => write!(f, "not signed by a device: {why}"),
            Self::Bad(why) => f.write_str(why),
        }
    }
}

/// This device's key, and the keys of the person's devices it trusts.
///
/// Kept in the app's folder, never in a vault: `device-key` (its secret
/// key, readable by this user only) and `my-devices.json` (the others).
#[derive(Clone)]
pub struct DeviceKeys {
    dir: PathBuf,
    name: String,
    key: Arc<SecretKey>,
    trusted: Arc<Mutex<Vec<DeviceName>>>,
}

impl std::fmt::Debug for DeviceKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeviceKeys")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl DeviceKeys {
    /// This device's keys in `dir` (the app's folder), called `name`: its
    /// key made the first time.
    ///
    /// # Errors
    ///
    /// Its key cannot be read or made.
    pub fn open(dir: &Path, name: &str) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let at = dir.join("device-key");
        let key = match std::fs::read_to_string(&at) {
            Ok(text) => SecretKey::parse(&text).map_err(|e| e.to_string())?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let key = SecretKey::generate().map_err(|e| e.to_string())?;
                write_private(&at, key.secret_file().as_bytes())?;
                key
            }
            Err(e) => return Err(format!("{}: {e}", at.display())),
        };
        let trusted = std::fs::read_to_string(dir.join("my-devices.json"))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Ok(Self {
            dir: dir.to_path_buf(),
            name: name.to_owned(),
            key: Arc::new(key),
            trusted: Arc::new(Mutex::new(trusted)),
        })
    }

    /// This device, as a record names it.
    #[must_use]
    pub fn me(&self) -> DeviceName {
        DeviceName {
            name: self.name.clone(),
            key: self.key.public().trim().to_owned(),
        }
    }

    /// Trusts `device` as one of this person's: records it signs apply
    /// here from now on. The person's to say, once a device.
    ///
    /// # Errors
    ///
    /// The list cannot be written.
    pub fn trust(&self, device: &DeviceName) -> Result<(), String> {
        let mut list = self.trusted.lock().unwrap_or_else(PoisonError::into_inner);
        if !list.iter().any(|d| d.key.trim() == device.key.trim()) {
            list.push(device.clone());
        }
        let text = serde_json::to_string_pretty(&*list).map_err(|e| e.to_string())?;
        drop(list);
        write_private(&self.dir.join("my-devices.json"), text.as_bytes())
    }

    /// The keys a record may be signed by here: this device's, and the
    /// person's devices it trusts.
    fn trust_set(&self) -> Trust {
        let me = self.me();
        self.trusted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .fold(Trust::none().with(me.name, me.key), |t, d| {
                t.with(d.name.clone(), d.key.clone())
            })
    }

    fn sign(&self, bytes: &[u8], person: &str) -> String {
        self.key.sign(bytes, &format!("person-installs {person}"))
    }
}

fn write_private(at: &Path, bytes: &[u8]) -> Result<(), String> {
    std::fs::write(at, bytes).map_err(|e| format!("{}: {e}", at.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(at, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// When this device last said or took the word on each id: a record's
/// entry applies only when it is later.
fn read_seen(installs_root: &Path) -> BTreeMap<String, u64> {
    std::fs::read_to_string(installs_root.join(".person-sync.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn write_seen(installs_root: &Path, seen: &BTreeMap<String, u64>) {
    if let Ok(text) = serde_json::to_string(seen) {
        let _ = std::fs::create_dir_all(installs_root);
        let _ = std::fs::write(installs_root.join(".person-sync.json"), text);
    }
}

/// What applying one entry did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Applied {
    /// Installed here at the record's version, its grants given.
    Installed { id: String, version: String },
    /// Removed here, as it was there.
    Removed { id: String },
    /// Its grants (and pin) made the record's: a revoke there, a revoke
    /// here.
    Granted { id: String },
    /// Nothing newer than this device's own last word.
    Here { id: String },
}

/// A person's installs kept in sync from this device: the vault, the
/// person, and this device's keys.
#[derive(Clone, Debug)]
pub struct PersonSync {
    pub root: PathBuf,
    pub person: String,
    pub keys: DeviceKeys,
}

impl Extensions {
    /// Keeps `sync`'s person's record up to date from now on: each install,
    /// removal or revoke made here is published at once
    /// ([`Self::publish_installs`]). Set once a vault is open and someone
    /// is signed in.
    pub fn sync_person(&self, sync: PersonSync) {
        // The hook once; the person and vault it publishes to, the latest.
        if self.set_person(sync) {
            let me = self.clone();
            self.on_installs_changed(move |id| {
                if let Some(sync) = me.person_sync() {
                    if let Err(error) = me.publish_installs(&sync, Some(id)) {
                        tracing::warn!(%error, ext.id = id, "ext.person_sync.publish");
                    }
                }
            });
        }
    }

    /// `person`'s record in the vault at `root`, checked: signed by this
    /// device or one of the person's devices it trusts, the person's own,
    /// no link, not too large. `Ok(None)` when there is none.
    ///
    /// # Errors
    ///
    /// [`RecordError`]: nothing of it may be applied.
    pub fn person_record(
        &self,
        root: &Path,
        person: &str,
        keys: &DeviceKeys,
    ) -> Result<Option<PersonInstalls>, RecordError> {
        if !is_person(person) {
            return Err(RecordError::Bad(format!("{person:?} is not an account id")));
        }
        let rel = record_path(person);
        let Some(path) = real(root, &rel, true).map_err(RecordError::Bad)? else {
            return Ok(None);
        };
        let size = std::fs::symlink_metadata(&path)
            .map_err(|e| RecordError::Bad(e.to_string()))?
            .len();
        if size > MAX_RECORD {
            return Err(RecordError::Bad(format!(
                "{rel}: {size} bytes, more than {MAX_RECORD}"
            )));
        }
        let bytes = std::fs::read(&path).map_err(|e| RecordError::Bad(e.to_string()))?;
        let text = String::from_utf8(bytes.clone()).map_err(|e| RecordError::Bad(e.to_string()))?;
        let record = PersonInstalls::from_toml(&text)
            .map_err(|e| RecordError::Bad(format!("{rel}: {e}")))?;
        let sig_rel = signature_path(person);
        let sig = match real(root, &sig_rel, true).map_err(RecordError::Bad)? {
            Some(p) if std::fs::symlink_metadata(&p).is_ok_and(|m| m.len() <= 4096) => {
                std::fs::read_to_string(&p).map_err(|e| RecordError::Unsigned(e.to_string()))?
            }
            _ => return Err(RecordError::Unsigned(format!("no {sig_rel}"))),
        };
        let (_, comment) = match verify_detached(&bytes, &sig, &keys.trust_set()) {
            Ok(ok) => ok,
            Err(architect_extension_archive::ArchiveError::UntrustedKey { .. }) => {
                return Err(RecordError::UntrustedDevice {
                    device: record.device,
                });
            }
            Err(e) => return Err(RecordError::Unsigned(e.to_string())),
        };
        if record.person != person || comment != format!("person-installs {person}") {
            return Err(RecordError::Bad(format!("{rel} is not {person}'s record")));
        }
        Ok(Some(record))
    }

    /// Writes what changed here into `sync`'s person's record, signed by
    /// this device: `only` (an id just installed, removed or revoked) or,
    /// when `None`, every install here. Each signed archive installed is
    /// copied into the vault's carried folder.
    ///
    /// # Errors
    ///
    /// The record there is signed by a device this one does not trust (it
    /// is not written over), or a file cannot be written.
    pub fn publish_installs(
        &self,
        sync: &PersonSync,
        only: Option<&str>,
    ) -> Result<PersonInstalls, String> {
        let PersonSync { root, person, keys } = sync;
        let mut record = match self.person_record(root, person, keys) {
            Ok(r) => r.unwrap_or_default(),
            // An unsigned one (a person's first, or a vault edited by
            // hand) is started again; one signed by a device not trusted
            // here is not written over.
            Err(RecordError::Unsigned(_)) => PersonInstalls::default(),
            Err(e) => return Err(e.to_string()),
        };
        record.person.clone_from(person);
        record.device = Some(keys.me());
        let at = now_ms();
        let mut seen = read_seen(self.installs().root());
        let ids: Vec<String> = only.map_or_else(
            || {
                self.installs()
                    .list()
                    .into_iter()
                    .filter(crate::install::Install::is_extension)
                    .map(|i| i.id)
                    .collect()
            },
            |id| vec![id.to_owned()],
        );
        let grants = self.host().grants();
        for id in ids {
            // Later than any word on it so far, whatever the clock says.
            let at = record
                .entry(&id)
                .map(|e| e.at_ms)
                .into_iter()
                .chain(seen.get(&id).copied())
                .max()
                .map_or(at, |last| at.max(last.saturating_add(1)));
            let entry = if let Some(install) = self.installs().get(&id) {
                let (Some(key), Some(bytes)) =
                    (install.signer_key.clone(), self.installs().archive_of(&id))
                else {
                    // Not from a signed archive (a folder registry's, a
                    // development copy): this device's alone.
                    continue;
                };
                let archive = format!("{id}-{}.archext", install.version);
                real_write(root, &format!("{CARRIED}/{archive}"), &bytes)?;
                let signer = self
                    .installs()
                    .trust_with(&self.sources())
                    .keys()
                    .iter()
                    .find(|k| k.key.trim() == key.trim())
                    .map(|k| k.name.clone())
                    .unwrap_or_default();
                SyncedInstall {
                    id: id.clone(),
                    version: install.version.to_string(),
                    pin: install.pin.clone(),
                    signer,
                    signer_key: key,
                    archive,
                    sha256: hex::encode(sha2::Sha256::digest(&bytes)),
                    at_ms: at,
                    removed: false,
                    grants: grants
                        .as_ref()
                        .map(|g| {
                            g.list()
                                .into_iter()
                                .filter(|g| g.id == id)
                                .map(|g| SyncedGrant {
                                    scope: g.scope,
                                    capabilities: g.capabilities,
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                }
            } else if record.entry(&id).is_some_and(|e| !e.removed) {
                // Not here any more: removed, said so.
                SyncedInstall {
                    id: id.clone(),
                    version: String::new(),
                    pin: None,
                    signer: String::new(),
                    signer_key: String::new(),
                    archive: String::new(),
                    sha256: String::new(),
                    at_ms: at,
                    removed: true,
                    grants: Vec::new(),
                }
            } else {
                continue;
            };
            record.put(entry);
            seen.insert(id, at);
        }
        let text = record.to_toml()?;
        real_write(root, &record_path(person), text.as_bytes())?;
        real_write(
            root,
            &signature_path(person),
            keys.sign(text.as_bytes(), person).as_bytes(),
        )?;
        write_seen(self.installs().root(), &seen);
        Ok(record)
    }

    /// Applies here what `sync`'s person said on their other devices:
    /// each entry later than this device's own last word on its id is
    /// installed (with its grants, no prompt), removed, or has its grants
    /// made the record's. Each outcome, or why not.
    ///
    /// # Errors
    ///
    /// The record is not signed by this person's devices this device
    /// trusts, is another person's, a link, or does not read: nothing of
    /// it is applied.
    pub fn apply_installs(
        &self,
        sync: &PersonSync,
    ) -> Result<Vec<Result<Applied, (String, InstallError)>>, RecordError> {
        let Some(record) = self.person_record(&sync.root, &sync.person, &sync.keys)? else {
            return Ok(Vec::new());
        };
        let mut seen = read_seen(self.installs().root());
        let out = record
            .installs
            .iter()
            .map(|s| {
                if seen.get(&s.id).is_some_and(|at| *at >= s.at_ms) {
                    return Ok(Applied::Here { id: s.id.clone() });
                }
                let done = self.apply_one(&sync.root, s);
                if done.is_ok() {
                    seen.insert(s.id.clone(), s.at_ms);
                }
                done.map_err(|e| (s.id.clone(), e))
            })
            .collect();
        write_seen(self.installs().root(), &seen);
        let _ = self.sync();
        Ok(out)
    }

    /// Makes this device's grants of `id` exactly `wanted`.
    fn set_grants(&self, id: &str, version: &str, wanted: &[SyncedGrant]) {
        let Some(grants) = self.host().grants() else {
            return;
        };
        grants.revoke(id, None);
        for g in wanted {
            grants.grant(id, version, g.capabilities.clone(), g.scope.clone(), "sync");
        }
        self.host().grants_changed(id);
    }

    fn apply_one(&self, root: &Path, s: &SyncedInstall) -> Result<Applied, InstallError> {
        if s.removed {
            // Removed without telling: it is the other device's word.
            self.installs().uninstall(&s.id)?;
            self.set_grants(&s.id, "", &[]);
            return Ok(Applied::Removed { id: s.id.clone() });
        }
        let version =
            Version::parse(&s.version).map_err(|e| InstallError::Mismatch(e.to_string()))?;
        if self
            .installs()
            .get(&s.id)
            .is_some_and(|i| i.version == version)
        {
            self.set_grants(&s.id, &s.version, &s.grants);
            if let Some(pin) = &s.pin {
                self.installs().set_pin(&s.id, Some(pin))?;
            }
            return Ok(Applied::Granted { id: s.id.clone() });
        }
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
        // The key the person's own device named (the record is theirs:
        // checked above), and only it, for this id.
        let named = Trust::none().with(
            if s.signer.is_empty() {
                "the key your device named".to_owned()
            } else {
                s.signer.clone()
            },
            s.signer_key.clone(),
        );
        let p = architect_extension_archive::peek(&bytes, &named, MAX_PEEK)?;
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
        let install = self
            .installs()
            .install_bytes(bytes, &listing, "your other device", named)?;
        // Granted only once it installed (it loads with them at the sync).
        self.set_grants(&install.id, &s.version, &s.grants);
        if let Some(pin) = &s.pin {
            self.installs().set_pin(&install.id, Some(pin))?;
        }
        Ok(Applied::Installed {
            id: install.id,
            version: install.version.to_string(),
        })
    }
}
