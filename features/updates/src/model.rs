//! The updater's model: channels, the install plan, the state machine.
//!
//! Honest: no I/O. What the process's environment says comes in as an
//! [`Env`]; the platform as a [`Platform`]; the edge reads both
//! (`Env::from_vars`, `Platform::this`).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// The env the Nix launcher sets: where a downloaded update goes.
pub const UPDATE_DIR_ENV: &str = "ARCHITECT_UPDATE_DIR";

/// The env an `AppImage`'s runtime sets: the image being run.
pub const APPIMAGE_ENV: &str = "APPIMAGE";

/// The file a downloaded update is, in the update dir and on a release.
pub const LINUX_ASSET: &str = "Architect-linux-x86_64.AppImage";

/// The macOS disk image on a release.
pub const MACOS_ASSET: &str = "Architect-macos-arm64.dmg";

/// The file beside the downloaded image naming its version (`X.Y.Z`).
pub const VERSION_FILE: &str = "version";

/// Which releases the updater follows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Channel {
    /// Tagged `vX.Y.Z` releases, never a prerelease.
    #[default]
    Stable,
    /// Nightly prereleases (#523 (d)): not available yet.
    Nightly,
}

impl Channel {
    /// A setting's value (`stable`, `nightly`); anything else is Stable.
    #[must_use]
    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "nightly" => Self::Nightly,
            _ => Self::Stable,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
        }
    }

    /// Whether this build can follow it; why not, when it cannot.
    ///
    /// # Errors
    ///
    /// Nightly: no nightly releases exist yet.
    pub const fn available(self) -> Result<(), &'static str> {
        match self {
            Self::Stable => Ok(()),
            Self::Nightly => Err(
                "the nightly channel is not available yet: there are no nightly releases to follow; \
                 switch the update channel to Stable",
            ),
        }
    }
}

/// The OS and CPU this build runs on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Platform {
    /// `std::env::consts::OS`: `linux`, `macos`, `windows`.
    pub os: String,
    /// `std::env::consts::ARCH`: `x86_64`, `aarch64`.
    pub arch: String,
}

impl Platform {
    /// This build's.
    #[must_use]
    pub fn this() -> Self {
        Self::new(std::env::consts::OS, std::env::consts::ARCH)
    }

    #[must_use]
    pub fn new(os: &str, arch: &str) -> Self {
        Self {
            os: os.to_owned(),
            arch: arch.to_owned(),
        }
    }

    /// The release asset that finds this platform's release.
    ///
    /// Linux's `AppImage` elsewhere too: only to find the release to
    /// report; nothing but Linux x86-64 installs.
    #[must_use]
    pub fn asset(&self) -> &'static str {
        if self.os == "macos" {
            MACOS_ASSET
        } else {
            LINUX_ASSET
        }
    }

    /// Whether the release's `AppImage` runs here.
    #[must_use]
    pub fn runs_appimage(&self) -> bool {
        self.os == "linux" && self.arch == "x86_64"
    }
}

/// What the process's environment says about where it runs from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Env {
    /// `$ARCHITECT_UPDATE_DIR` (the Nix launcher's), when set and not empty.
    pub update_dir: Option<PathBuf>,
    /// `$APPIMAGE`, when set and not empty.
    pub appimage: Option<PathBuf>,
    /// Whether `appimage` can be replaced in place by this user.
    pub appimage_writable: bool,
}

impl Env {
    /// Read from `var` (`std::env::var_os` at the edge, a map in a test),
    /// asking `writable` about the `AppImage`.
    pub fn from_vars(
        var: impl Fn(&str) -> Option<OsString>,
        writable: impl Fn(&Path) -> bool,
    ) -> Self {
        let path = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let appimage = path(APPIMAGE_ENV);
        let appimage_writable = appimage.as_deref().is_some_and(writable);
        Self {
            update_dir: path(UPDATE_DIR_ENV),
            appimage,
            appimage_writable,
        }
    }
}

/// Why an update is reported, not installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "why", rename_all = "snake_case")]
pub enum ReportOnly {
    /// Not stamped with a release version: a cargo run or a dev build.
    DevelopmentBuild,
    /// macOS, Windows, or a CPU the `AppImage` is not built for.
    Platform { os: String, arch: String },
    /// Not run from an `AppImage` (a distro package, a bare binary).
    NoAppImage,
    /// The `AppImage` cannot be replaced by this user.
    NotWritable { appimage: PathBuf },
}

impl ReportOnly {
    /// In words, for the state and the settings row.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::DevelopmentBuild => {
                "this is a development build (no release version stamped in), so it does not update itself"
                    .to_owned()
            }
            Self::Platform { os, arch } => format!(
                "installing updates on {os} ({arch}) is not supported yet: download the new release"
            ),
            Self::NoAppImage => {
                "Architect is not running from an AppImage, so it cannot replace itself: download the new release"
                    .to_owned()
            }
            Self::NotWritable { appimage } => format!(
                "{} cannot be replaced by this user: download the new release",
                appimage.display()
            ),
        }
    }
}

/// Where a verified update goes, decided once per run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "plan", rename_all = "snake_case")]
pub enum Plan {
    /// The Nix launcher's dir: `<dir>/Architect-linux-x86_64.AppImage`,
    /// then `<dir>/version`; `$APPIMAGE` untouched.
    UpdateDir { dir: PathBuf },
    /// A plain `AppImage` download, replaced where it is.
    InPlace { appimage: PathBuf },
    /// Report the update and where to download it; install nothing.
    ReportOnly(ReportOnly),
}

impl Plan {
    /// Whether it installs anything.
    #[must_use]
    pub const fn installs(&self) -> bool {
        !matches!(self, Self::ReportOnly(_))
    }
}

/// Where an update for this build goes.
///
/// A development build, then the platform, then the update dir, then a
/// writable `$APPIMAGE`, else report only.
#[must_use]
pub fn plan(env: &Env, platform: &Platform, released: bool) -> Plan {
    if !released {
        return Plan::ReportOnly(ReportOnly::DevelopmentBuild);
    }
    if !platform.runs_appimage() {
        return Plan::ReportOnly(ReportOnly::Platform {
            os: platform.os.clone(),
            arch: platform.arch.clone(),
        });
    }
    if let Some(dir) = &env.update_dir {
        return Plan::UpdateDir { dir: dir.clone() };
    }
    match &env.appimage {
        Some(appimage) if env.appimage_writable => Plan::InPlace {
            appimage: appimage.clone(),
        },
        Some(appimage) => Plan::ReportOnly(ReportOnly::NotWritable {
            appimage: appimage.clone(),
        }),
        None => Plan::ReportOnly(ReportOnly::NoAppImage),
    }
}

/// The stamped release version, if it is one (`X.Y.Z`, a `v` dropped).
#[must_use]
pub fn release_version(stamp: Option<&str>) -> Option<String> {
    let v = stamp?.trim().trim_start_matches('v');
    let parts: Vec<&str> = v.split('.').collect();
    let ok = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    ok.then(|| v.to_owned())
}

/// Where the updater is.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum State {
    /// Not checked yet.
    #[default]
    Idle,
    /// Not stamped with a release version (a cargo run, a dev build): it
    /// never checks, offers or installs anything.
    DevelopmentBuild,
    Checking,
    /// The running version is the newest release (`newest` none: no
    /// release yet).
    UpToDate {
        newest: Option<String>,
    },
    /// A newer release exists.
    Available {
        version: String,
        /// The release's page: its notes and every download.
        notes_url: String,
        /// Whether its sums are signed; an unsigned one is never installed.
        signed: bool,
        /// Where it would go.
        plan: Plan,
    },
    /// Downloading and verifying `version`.
    Downloading {
        version: String,
        received: u64,
        total: Option<u64>,
    },
    /// `version` is installed; the next start runs it.
    Ready {
        version: String,
        plan: Plan,
    },
    /// The last check or install failed; nothing changed.
    Failed {
        reason: String,
    },
}

/// What the status bar shows for a state: its text and what a press runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusLine {
    pub text: String,
    pub hint: String,
    pub action: &'static str,
}

/// The status bar item for `state`; none when there is nothing to say.
#[must_use]
pub fn status_line(state: &State) -> Option<StatusLine> {
    match state {
        State::Available {
            version,
            signed,
            plan,
            ..
        } => {
            let (hint, action) = if !*signed {
                (
                    format!("{version} is not signed, so it is not installed; open its page"),
                    crate::actions::PAGE,
                )
            } else if plan.installs() {
                (
                    format!("Download and install {version}"),
                    crate::actions::INSTALL,
                )
            } else {
                (
                    "Open the release's download page".to_owned(),
                    crate::actions::PAGE,
                )
            };
            Some(StatusLine {
                text: format!("Update {version} available"),
                hint,
                action,
            })
        }
        State::Downloading {
            version,
            received,
            total,
        } => Some(StatusLine {
            text: percent(*received, *total).map_or_else(
                || format!("Downloading {version}…"),
                |p| format!("Downloading {version}… {p}%"),
            ),
            hint: "Downloading and verifying the update".to_owned(),
            action: crate::actions::STATE,
        }),
        State::Ready { version, plan } => Some(StatusLine {
            text: "Restart to finish".to_owned(),
            hint: match plan {
                Plan::InPlace { .. } => format!("Restart Architect to run {version}"),
                _ => format!("Quit Architect and open it again to run {version}"),
            },
            action: crate::actions::RESTART,
        }),
        State::Idle
        | State::DevelopmentBuild
        | State::Checking
        | State::UpToDate { .. }
        | State::Failed { .. } => None,
    }
}

/// `received` of `total` as a whole percent.
#[must_use]
pub fn percent(received: u64, total: Option<u64>) -> Option<u64> {
    let total = total.filter(|t| *t > 0)?;
    received
        .saturating_mul(100)
        .checked_div(total)
        .map(|p| p.min(100))
}
