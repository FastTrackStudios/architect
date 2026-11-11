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

/// The env an `AppImage`'s runtime sets: where the image is mounted.
pub const APPDIR_ENV: &str = "APPDIR";

/// Every release's page; a release page must be under it.
pub const RELEASES_PAGE: &str = "https://github.com/FastTrackStudios/architect/releases";

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
    /// Whether `appimage` is the image this process runs from.
    ///
    /// `$APPDIR` is set, the running binary is under it, and the file is
    /// named `*.AppImage`. An `$APPIMAGE` inherited from another app's
    /// image (a terminal opened inside it) is not.
    pub appimage_ours: bool,
    /// Whether `appimage` can be replaced in place by this user.
    pub appimage_writable: bool,
}

impl Env {
    /// Read from `var` (`std::env::var_os` at the edge, a map in a test).
    ///
    /// `exe` is the running binary (`current_exe`, symlinks resolved);
    /// `writable` is asked about the `AppImage` only once it is ours.
    pub fn from_vars(
        var: impl Fn(&str) -> Option<OsString>,
        exe: Option<&Path>,
        writable: impl Fn(&Path) -> bool,
    ) -> Self {
        let path = |name: &str| var(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        let appimage = path(APPIMAGE_ENV);
        let appdir = path(APPDIR_ENV);
        let appimage_ours = appimage.as_deref().is_some_and(|image| {
            let named = image
                .file_name()
                .is_some_and(|n| n.to_string_lossy().ends_with(".AppImage"));
            let inside = matches!((&appdir, exe), (Some(dir), Some(exe)) if exe.starts_with(dir));
            named && inside
        });
        let appimage_writable = appimage_ours && appimage.as_deref().is_some_and(writable);
        Self {
            update_dir: path(UPDATE_DIR_ENV),
            appimage,
            appimage_ours,
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
    /// `$APPIMAGE` names an image this process does not run from.
    ForeignAppImage { appimage: PathBuf },
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
            Self::ForeignAppImage { appimage } => format!(
                "$APPIMAGE ({}) is not the image Architect runs from, so it is left alone: download the new release",
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
        Some(appimage) if !env.appimage_ours => Plan::ReportOnly(ReportOnly::ForeignAppImage {
            appimage: appimage.clone(),
        }),
        Some(appimage) if env.appimage_writable => Plan::InPlace {
            appimage: appimage.clone(),
        },
        Some(appimage) => Plan::ReportOnly(ReportOnly::NotWritable {
            appimage: appimage.clone(),
        }),
        None => Plan::ReportOnly(ReportOnly::NoAppImage),
    }
}

/// The stamped release version, if it is one (a `v` dropped).
///
/// `X.Y.Z`, or a nightly's `X.Y.Z-nightly.YYYYMMDD`: the two forms the
/// Nix launcher ranks (`nix/desktop/choose.sh`). Anything else is none.
#[must_use]
pub fn release_version(stamp: Option<&str>) -> Option<String> {
    let v = stamp?.trim().trim_start_matches('v');
    version_key(v).map(|_| v.to_owned())
}

/// A version's rank: `(X, Y, Z, 1 for a release or 0 for a nightly,
/// the nightly's date)`, as `nix/desktop/choose.sh` `key` ranks them.
#[must_use]
pub fn version_key(v: &str) -> Option<(u64, u64, u64, u8, u64)> {
    let number = |p: &str| {
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse::<u64>().ok())
            .flatten()
    };
    let (core, nightly) = match v.split_once('-') {
        Some((core, tail)) => {
            let date = tail.strip_prefix("nightly.")?;
            (date.len() == 8).then_some(())?;
            (core, Some(number(date)?))
        }
        None => (v, None),
    };
    let mut parts = core.split('.');
    let x = number(parts.next()?)?;
    let y = number(parts.next()?)?;
    let z = number(parts.next()?)?;
    if parts.next().is_some() {
        return None;
    }
    Some(nightly.map_or((x, y, z, 1, 0), |date| (x, y, z, 0, date)))
}

/// Whether `candidate` ranks above `current` (both must parse): a release
/// beats a nightly of the same `X.Y.Z`.
#[must_use]
pub fn newer(candidate: &str, current: &str) -> bool {
    matches!((version_key(candidate), version_key(current)), (Some(a), Some(b)) if a > b)
}

/// `url` when it is one of Architect's release pages, else
/// [`RELEASES_PAGE`]: a page URL comes from unsigned API JSON.
#[must_use]
pub fn release_page(url: &str) -> &str {
    let under = format!("{RELEASES_PAGE}/");
    let safe = url.strip_prefix(&under).is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
            && !rest.contains("..")
    });
    if safe { url } else { RELEASES_PAGE }
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

/// The updater's state with what goes with it, changed only through its
/// transitions: each is one compare-and-set under the caller's lock.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    pub state: State,
    /// RFC 3339: the last check that reached the releases API.
    pub last_check: Option<String>,
    pub channel: Channel,
}

/// What [`Machine::begin_install`] decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Begin {
    /// Download and install `version` per `plan`; the state is
    /// Downloading.
    Start { version: String, plan: Plan },
    /// An install is already under way or done; nothing to start.
    Already,
}

impl Machine {
    /// Idle, on Stable: for a `static`.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            state: State::Idle,
            last_check: None,
            channel: Channel::Stable,
        }
    }

    /// Start a check: false (and nothing changed) while a check runs, an
    /// install is under way or done, or in a development build.
    pub fn begin_check(&mut self) -> bool {
        if matches!(
            self.state,
            State::Checking
                | State::Downloading { .. }
                | State::Ready { .. }
                | State::DevelopmentBuild
        ) {
            return false;
        }
        self.state = State::Checking;
        true
    }

    /// A check's answer, kept only while the state is still that check:
    /// a late check never replaces an install.
    pub fn finish_check(&mut self, state: State, at: Option<String>) -> bool {
        if self.state != State::Checking {
            return false;
        }
        if at.is_some() {
            self.last_check = at;
        }
        self.state = state;
        true
    }

    /// Start installing the available update; only from Available.
    ///
    /// # Errors
    ///
    /// Nothing is available, or it would only be reported (the reason and
    /// its page).
    pub fn begin_install(&mut self) -> Result<Begin, String> {
        let (version, plan) = match &self.state {
            State::Available {
                version,
                plan,
                notes_url,
                ..
            } => {
                if let Plan::ReportOnly(why) = plan {
                    return Err(format!("{}: {}", why.reason(), release_page(notes_url)));
                }
                (version.clone(), plan.clone())
            }
            State::Downloading { .. } | State::Ready { .. } => return Ok(Begin::Already),
            State::DevelopmentBuild => return Err(ReportOnly::DevelopmentBuild.reason()),
            _ => return Err("no update is available: check first (UPDATES_CHECK)".to_owned()),
        };
        self.state = State::Downloading {
            version: version.clone(),
            received: 0,
            total: None,
        };
        Ok(Begin::Start { version, plan })
    }

    /// Bytes of `version` received; answers whether the whole percent
    /// moved (worth redrawing the status item).
    pub fn progress(&mut self, version: &str, received: u64, total: Option<u64>) -> bool {
        let State::Downloading {
            version: v,
            received: r,
            total: t,
        } = &mut self.state
        else {
            return false;
        };
        if v != version {
            return false;
        }
        let moved = percent(*r, *t) != percent(received, total);
        *r = received;
        *t = total;
        moved
    }

    /// An install's end (Ready or Failed), kept only while Downloading.
    pub fn finish_install(&mut self, state: State) -> bool {
        if !matches!(self.state, State::Downloading { .. }) {
            return false;
        }
        self.state = state;
        true
    }
}
