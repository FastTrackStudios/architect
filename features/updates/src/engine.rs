//! The updater's work (feature `release`): check for a release, then
//! download, verify and install it per the [`Plan`].
//!
//! Every input is a parameter: the releases API (`api_base`), the key,
//! the running version, the platform, the env and the directories. The
//! extension reads them at the edge; a test passes its own.

use std::path::Path;

use architect::release::{self, Candidate, ReleaseError, Updater};

use crate::model::{Channel, Env, LINUX_ASSET, Plan, Platform, State, VERSION_FILE, newer, plan};

/// Writes the `version` file: `(dir, name, text)`.
pub type WriteVersion = fn(&Path, &str, &str) -> Result<(), ReleaseError>;

/// Which app's releases, and with whose key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Source {
    /// The name its release workflow signs with (`architect`): the
    /// signature's trusted comment is `<app> <tag> SHA256SUMS`.
    pub app: &'static str,
    /// `owner/name` on GitHub.
    pub repo: &'static str,
    /// The release key's public half; `None` checks and never installs.
    pub key: Option<&'static str>,
}

/// The Architect app's releases, signed with the org key.
pub const ARCHITECT: Source = Source {
    app: "architect",
    repo: "FastTrackStudios/architect",
    key: Some(release::ORG_KEY),
};

/// Why an install did not happen; nothing was changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    /// In words, for the person.
    pub reason: String,
    /// A release that failed verification (unsigned, a bad signature, a
    /// checksum mismatch): worth an error in the log, unlike a network
    /// failure.
    pub refused: bool,
}

impl Failure {
    fn plain(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            refused: false,
        }
    }
}

impl From<ReleaseError> for Failure {
    fn from(e: ReleaseError) -> Self {
        let refused = matches!(
            e,
            ReleaseError::Unsigned { .. }
                | ReleaseError::BadSignature { .. }
                | ReleaseError::BadKey(_)
                | ReleaseError::NotListed { .. }
                | ReleaseError::ChecksumMismatch { .. }
                | ReleaseError::NoSigningKey
                | ReleaseError::TooLarge { .. }
        );
        let reason = match e {
            ReleaseError::Unsigned { tag } => format!(
                "{tag} is not signed (it carries no {}), so it is not installed; nothing changed",
                release::SIG_ASSET
            ),
            other => other.to_string(),
        };
        Self { reason, refused }
    }
}

/// One app's updater on this machine.
#[derive(Debug, Clone)]
pub struct Engine {
    updater: Updater,
    key: Option<String>,
    current: Option<String>,
    platform: Platform,
    write_version: WriteVersion,
}

impl Engine {
    /// `current` is the stamped release version (none: a development
    /// build, which never checks or installs).
    #[must_use]
    pub fn new(source: &Source, current: Option<String>, platform: Platform) -> Self {
        Self {
            updater: Updater::new(source.app, source.repo, platform.asset()),
            key: source.key.map(str::to_owned),
            current,
            platform,
            write_version: release::write_atomic,
        }
    }

    /// The `version` file's writer, in place of [`release::write_atomic`]:
    /// a test's failing one.
    #[doc(hidden)]
    #[must_use]
    pub fn write_version_with(mut self, write: WriteVersion) -> Self {
        self.write_version = write;
        self
    }

    /// The caps on the image and the small files, in place of
    /// `architect::release`'s: a test's smaller ones.
    #[doc(hidden)]
    #[must_use]
    pub fn limits(mut self, asset: u64, small: u64) -> Self {
        self.updater = self.updater.limits(asset, small);
        self
    }

    /// The releases API base (default GitHub's); a test's local server.
    #[must_use]
    pub fn api_base(mut self, base: impl Into<String>) -> Self {
        self.updater = self.updater.api_base(base);
        self
    }

    /// The release key's public half, in place of the source's.
    #[must_use]
    pub fn key(mut self, key: Option<String>) -> Self {
        self.key = key;
        self
    }

    /// The running release version; none for a development build.
    #[must_use]
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Where an update would go with `env`.
    #[must_use]
    pub fn plan(&self, env: &Env) -> Plan {
        plan(env, &self.platform, self.current.is_some())
    }

    /// Ask for the newest release on `channel`: the state after.
    ///
    /// A development build answers at once without asking; a failure
    /// is [`State::Failed`], never an error.
    pub async fn check(&self, channel: Channel, env: &Env) -> State {
        let Some(current) = &self.current else {
            return State::DevelopmentBuild;
        };
        if let Err(why) = channel.available() {
            return State::Failed {
                reason: why.to_owned(),
            };
        }
        match self.updater.find(None).await {
            Err(e) => State::Failed {
                reason: e.to_string(),
            },
            Ok(None) => State::UpToDate { newest: None },
            Ok(Some(c)) if newer(&c.version, current) => State::Available {
                signed: c.sig_url.is_some(),
                version: c.version,
                notes_url: c.notes_url,
                plan: self.plan(env),
            },
            Ok(Some(c)) => State::UpToDate {
                newest: Some(c.version),
            },
        }
    }

    /// Download `version`, verify it, and install it per `plan`.
    ///
    /// `progress` hears `(received, total)`. Answers [`State::Ready`];
    /// on any failure before the image is renamed in, nothing changed. In
    /// the update dir an old `version` is removed before the new image goes
    /// in, so when writing the new `version` fails the folder holds an
    /// image with no `version`, which the launcher never runs
    /// (`nix/desktop/choose.sh`: no version means the store's image).
    ///
    /// # Errors
    ///
    /// A development build, a report-only plan, no such release, an
    /// unsigned or badly signed release, a checksum mismatch, a network or
    /// file failure.
    pub async fn install(
        &self,
        version: &str,
        plan: &Plan,
        progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
    ) -> Result<State, Failure> {
        let current = self
            .current
            .as_deref()
            .ok_or_else(|| Failure::plain(crate::model::ReportOnly::DevelopmentBuild.reason()))?;
        let (dir, dest) = match plan {
            Plan::ReportOnly(why) => return Err(Failure::plain(why.reason())),
            Plan::UpdateDir { dir } => {
                make_private_dir(dir)?;
                (dir.clone(), dir.join(LINUX_ASSET))
            }
            Plan::InPlace { appimage } => {
                let dir = appimage.parent().ok_or_else(|| {
                    Failure::plain(format!("{} has no folder", appimage.display()))
                })?;
                (dir.to_owned(), appimage.clone())
            }
        };
        if !newer(version, current) {
            return Err(Failure::plain(format!(
                "{version} is not newer than the running {current}; nothing changed"
            )));
        }
        let candidate = self.find(version).await?;
        let staged = self
            .updater
            .fetch_verified(&candidate, self.key.as_deref(), &dir, progress)
            .await?;
        let update_dir = matches!(plan, Plan::UpdateDir { .. });
        if update_dir {
            // No `version` while the image changes: an old one would name
            // the new image with the old version.
            remove_if_present(&dir.join(VERSION_FILE))?;
            sync_dir(&dir)?;
        }
        release::persist_executable(staged, &dest)?;
        sync_dir(&dir)?;
        if update_dir {
            (self.write_version)(&dir, VERSION_FILE, &format!("{}\n", candidate.version))?;
            sync_dir(&dir)?;
        }
        Ok(State::Ready {
            version: candidate.version,
            plan: plan.clone(),
        })
    }

    async fn find(&self, version: &str) -> Result<Candidate, Failure> {
        self.updater.find(Some(version)).await?.ok_or_else(|| {
            Failure::plain(format!(
                "no release v{version} carries {} and {}",
                self.updater.asset(),
                release::SUMS_ASSET
            ))
        })
    }
}

fn remove_if_present(path: &Path) -> Result<(), Failure> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            Err(Failure::plain(format!("remove {}: {e}", path.display())))
        }
        _ => Ok(()),
    }
}

/// Make the renames in `dir` durable (a no-op where a folder cannot be
/// opened to sync).
fn sync_dir(dir: &Path) -> Result<(), Failure> {
    #[cfg(unix)]
    {
        std::fs::File::open(dir)
            .and_then(|d| d.sync_all())
            .map_err(|e| Failure::plain(format!("sync {}: {e}", dir.display())))?;
    }
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

/// `dir`, made owner-only when it is missing.
fn make_private_dir(dir: &Path) -> Result<(), Failure> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|e| Failure::plain(format!("make {}: {e}", dir.display())))
}

/// Whether this user can replace the file at `path` in place.
///
/// The edge's probe for [`Env::from_vars`]: a file outside `/nix/store`,
/// not read-only, whose folder takes a new file.
#[must_use]
pub fn writable(path: &Path) -> bool {
    if path.starts_with("/nix/store") {
        return false;
    }
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    meta.is_file()
        && !meta.permissions().readonly()
        && path
            .parent()
            .is_some_and(|dir| release::staging_in(dir, "probe").is_ok())
}
