//! Access keys for the S3 face: one per person or per device, in one org.
//!
//! A key is issued on the node (`architect-desktop serve s3-key issue`,
//! or [`S3Keys::issue`]) for a member (`user:<id>`) or an enrolled device
//! (`device:<endpoint>`), with a role at most the issuer's. The secret
//! is kept in the agent's data dir (`s3-keys.json`, owner-only): `SigV4`
//! needs it on both sides. What a key may do is decided on every request:
//! its role, lowered to what the holder holds now ([`KeyRoles`]: a
//! revoked device or a member who left gets nothing), then the files
//! feature's capabilities for that role (`access::role_capabilities`,
//! `access::resolve`), as `FilesServer`'s `Acting` does.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use architect_orgs::Role;
use facet::Facet;
use rand::RngCore as _;

/// Who a key acts for.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(C)]
pub enum KeyHolder {
    /// A member of the org, by their user id at the issuer.
    User(String),
    /// A device, by its iroh endpoint (as `Admission` keys it).
    Device(String),
}

impl KeyHolder {
    /// `user:<id>` or `device:<endpoint>`.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s.split_once(':')? {
            ("user", id) if !id.is_empty() => Some(Self::User(id.to_owned())),
            ("device", ep) if !ep.is_empty() => Some(Self::Device(ep.to_owned())),
            _ => None,
        }
    }

    /// The user id a version is saved by, and a grant is looked up for.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::User(id) => format!("user:{id}"),
            Self::Device(ep) => format!("device:{ep}"),
        }
    }
}

/// One access key.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct S3Key {
    /// `AKAR…`: 20 characters, as AWS's.
    pub access_key: String,
    /// 40 characters; never logged, shown once when issued.
    pub secret: String,
    /// The org (its folder under `files_dir`): the bucket it opens.
    pub org: String,
    pub holder: KeyHolder,
    /// The most it may do; the holder's role now may lower it.
    pub role: Role,
    pub label: String,
    pub created_us: i64,
}

impl S3Key {
    /// The key without its secret, to list.
    #[must_use]
    pub fn redacted(&self) -> Self {
        Self {
            secret: String::new(),
            ..self.clone()
        }
    }
}

#[derive(Debug, Clone, Default, Facet)]
struct KeyFile {
    keys: Vec<S3Key>,
}

/// The node's keys, in one file. Cheap to clone.
#[derive(Clone)]
pub struct S3Keys {
    path: PathBuf,
    lock: Arc<Mutex<()>>,
}

impl S3Keys {
    /// The keys in `path` (made on the first issue).
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Arc::default(),
        }
    }

    /// `<data>/s3-keys.json`.
    #[must_use]
    pub fn in_dir(data: &Path) -> Self {
        Self::new(data.join("s3-keys.json"))
    }

    fn read(&self) -> KeyFile {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|s| facet_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn write(&self, file: &KeyFile) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        let json = facet_json::to_string(file).map_err(std::io::Error::other)?;
        write_private(&tmp, json.as_bytes())?;
        std::fs::rename(&tmp, &self.path)
    }

    /// Issue a key for `holder` in `org` with `role`.
    ///
    /// # Errors
    ///
    /// The key file cannot be written.
    pub fn issue(
        &self,
        org: &str,
        holder: KeyHolder,
        role: Role,
        label: &str,
    ) -> std::io::Result<S3Key> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut file = self.read();
        let key = S3Key {
            access_key: format!("AKAR{}", token(16, UPPER)),
            secret: token(40, SECRET),
            org: org.to_owned(),
            holder,
            role,
            label: label.to_owned(),
            created_us: super::super::engine::now_us(),
        };
        file.keys.push(key.clone());
        self.write(&file)?;
        Ok(key)
    }

    /// Every key, secrets removed.
    #[must_use]
    pub fn list(&self) -> Vec<S3Key> {
        self.read().keys.iter().map(S3Key::redacted).collect()
    }

    /// Revoke `access_key`; whether it was there.
    ///
    /// # Errors
    ///
    /// The key file cannot be written.
    pub fn revoke(&self, access_key: &str) -> std::io::Result<bool> {
        let _held = self.lock.lock().unwrap_or_else(PoisonError::into_inner);
        let mut file = self.read();
        let before = file.keys.len();
        file.keys.retain(|k| k.access_key != access_key);
        let gone = file.keys.len() != before;
        if gone {
            self.write(&file)?;
        }
        Ok(gone)
    }

    /// The key `access_key`, with its secret.
    #[must_use]
    pub fn get(&self, access_key: &str) -> Option<S3Key> {
        self.read()
            .keys
            .into_iter()
            .find(|k| k.access_key == access_key)
    }
}

/// What a key's holder holds in its org now: `None` when nothing (a
/// device revoked, a member gone). The face uses the lesser of this and
/// the key's own role.
pub trait KeyRoles: Send + Sync + 'static {
    fn role_now(&self, key: &S3Key) -> Option<Role>;
}

/// Trust the role a key was issued with (a device's own agent, tests).
pub struct AsIssued;

impl KeyRoles for AsIssued {
    fn role_now(&self, key: &S3Key) -> Option<Role> {
        Some(key.role.clone())
    }
}

impl<F: Fn(&S3Key) -> Option<Role> + Send + Sync + 'static> KeyRoles for F {
    fn role_now(&self, key: &S3Key) -> Option<Role> {
        self(key)
    }
}

/// The lesser of two roles (`Role` sorts the strongest first).
#[must_use]
pub fn lesser(a: Role, b: Role) -> Role {
    std::cmp::max(a, b)
}

const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
const SECRET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn token(len: usize, alphabet: &[u8]) -> String {
    let mut bytes = vec![0u8; len];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
        .iter()
        .map(|b| {
            let i = usize::from(*b).checked_rem(alphabet.len()).unwrap_or(0);
            char::from(alphabet.get(i).copied().unwrap_or(b'A'))
        })
        .collect()
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_issued_listed_without_secrets_and_revoked() {
        let dir = tempfile::tempdir().unwrap();
        let keys = S3Keys::in_dir(dir.path());
        let k = keys
            .issue("fts", KeyHolder::User("u1".into()), Role::Member, "laptop")
            .unwrap();
        assert_eq!(k.access_key.len(), 20);
        assert_eq!(k.secret.len(), 40);
        assert_eq!(
            keys.get(&k.access_key).map(|g| g.secret),
            Some(k.secret.clone())
        );
        assert!(keys.list().iter().all(|l| l.secret.is_empty()));
        assert!(keys.revoke(&k.access_key).unwrap());
        assert!(keys.get(&k.access_key).is_none());
        assert_eq!(lesser(Role::Owner, Role::Viewer), Role::Viewer);
        assert_eq!(
            KeyHolder::parse("device:abc"),
            Some(KeyHolder::Device("abc".into()))
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(dir.path().join("s3-keys.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o077, 0);
        }
    }
}
