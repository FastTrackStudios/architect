//! The drive: a device's roots mounted as a real folder (feature `drive`,
//! Linux and macOS), so any app (a DAW, a file manager, a file picker,
//! `rsync`) reads and writes them.
//!
//! [`mount`] mounts [`fs::DriveFs`] at a folder (`~/Architect` by default,
//! the agent's config `drive`). Unmounting is clean on every exit:
//!
//! - **On drop** the session unmounts.
//! - **On a crash or `kill -9`** a guard process the mount started (the
//!   same binary, `drive-guard <mount>`) sees its pipe from the agent
//!   close and lazily unmounts, which is what fusermount's `auto_unmount`
//!   does without needing `user_allow_other`.
//! - **At start**, a mount left `Transport endpoint is not connected` is
//!   cleared ([`clear_stale`]); a live one (another agent) is refused.
//!
//! All fetching and evicting is in [`cache`].

pub mod agent;
pub mod cache;
pub mod fs;

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use tokio::runtime::Handle;

pub use crate::roots::DriveStatus;
pub use cache::{Cache, Fetch};
pub use fs::{DriveFs, XATTR_PINNED, XATTR_STATE};

use crate::sync::Engine;

/// The argument the guard process is started with.
pub const GUARD_ARG: &str = "drive-guard";

/// A mounted drive. Dropping it unmounts.
pub struct Mounted {
    mountpoint: PathBuf,
    session: Option<fuser::BackgroundSession>,
    guard: Option<Child>,
    fs: DriveFs,
}

impl Mounted {
    /// Where it is mounted.
    #[must_use]
    pub fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    /// Its cache.
    #[must_use]
    pub fn cache(&self) -> &Cache {
        self.fs.cache()
    }

    /// Unmount now.
    pub fn unmount(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if let Some(session) = self.session.take()
            && let Err(e) = session.umount_and_join()
        {
            tracing::warn!(%e, mount = %self.mountpoint.display(), "drive: unmount");
            lazy_unmount(&self.mountpoint);
        }
        if let Some(mut guard) = self.guard.take() {
            let _ = guard.kill();
            let _ = guard.wait();
        }
    }
}

impl Drop for Mounted {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Why the drive did not mount.
#[derive(Debug, thiserror::Error)]
pub enum MountError {
    #[error("{0} is already mounted by another agent")]
    Busy(String),
    #[error("mounting {path}: {source}")]
    Mount {
        path: String,
        source: std::io::Error,
    },
}

/// Mount `engine`'s roots at `mountpoint`.
///
/// The engine is called on `rt`; writes to unpinned files are staged in
/// `tmp`. Started from a binary
/// that answers [`GUARD_ARG`] with [`guard_main`], it also starts the
/// guard that unmounts after a crash.
///
/// # Errors
///
/// Another live mount is there, or the kernel refused the mount.
pub fn mount(
    engine: Engine,
    rt: Handle,
    mountpoint: &Path,
    tmp: PathBuf,
    guard: bool,
) -> Result<Mounted, MountError> {
    let at = |source| MountError::Mount {
        path: mountpoint.display().to_string(),
        source,
    };
    if cfg!(target_os = "macos") {
        return Err(at(std::io::Error::other(
            "this build mounts no FUSE on a Mac (fuser is built without macFUSE); the Mac's drive is the File Provider",
        )));
    }
    clear_stale(mountpoint)?;
    std::fs::create_dir_all(mountpoint).map_err(at)?;
    // The files are the owner's of the mount point (made just now, or theirs).
    let owner = std::fs::metadata(mountpoint).map_err(at)?;
    let (uid, gid) = (
        std::os::unix::fs::MetadataExt::uid(&owner),
        std::os::unix::fs::MetadataExt::gid(&owner),
    );
    let fs = DriveFs::new(engine, rt, tmp, uid, gid);
    let mut config = fuser::Config::default();
    config.mount_options = vec![
        fuser::MountOption::FSName("architect".to_owned()),
        fuser::MountOption::Subtype("architect".to_owned()),
        fuser::MountOption::NoAtime,
        fuser::MountOption::DefaultPermissions,
    ];
    // Finder shows the volume by this name (macFUSE / FUSE-T).
    if cfg!(target_os = "macos") {
        config
            .mount_options
            .push(fuser::MountOption::CUSTOM("volname=Architect".to_owned()));
    }
    config.n_threads = Some(4);
    let session = fuser::spawn_mount(fs.clone(), mountpoint, &config).map_err(at)?;
    let guard = guard.then(|| spawn_guard(mountpoint)).flatten();
    tracing::info!(mount = %mountpoint.display(), guarded = guard.is_some(), "drive: mounted");
    Ok(Mounted {
        mountpoint: mountpoint.to_owned(),
        session: Some(session),
        guard,
        fs,
    })
}

/// Start `<this binary> drive-guard <mount>` with a pipe from this process
/// on its stdin: when this process ends, however it ends, the pipe closes
/// and the guard unmounts.
fn spawn_guard(mountpoint: &Path) -> Option<Child> {
    let exe = std::env::current_exe().ok()?;
    Command::new(exe)
        .arg(GUARD_ARG)
        .arg(mountpoint)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(
            |e| tracing::warn!(%e, "drive: no guard; a crash leaves the mount for the next start"),
        )
        .ok()
}

/// The guard: wait for stdin to close (the agent ended), then unmount.
#[must_use]
pub fn guard_main(mountpoint: &Path) -> std::process::ExitCode {
    let mut sink = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut sink);
    if is_mounted(mountpoint) {
        lazy_unmount(mountpoint);
    }
    std::process::ExitCode::SUCCESS
}

/// Whether `mountpoint` is a FUSE mount now (Linux: `/proc/self/mountinfo`).
#[must_use]
pub fn is_mounted(mountpoint: &Path) -> bool {
    let target = mountpoint.to_string_lossy().replace(' ', "\\040");
    std::fs::read_to_string("/proc/self/mountinfo").map_or_else(
        |_| {
            // macOS: `mount` lists it.
            Command::new("mount").output().is_ok_and(|o| {
                String::from_utf8_lossy(&o.stdout).contains(&*mountpoint.to_string_lossy())
            })
        },
        |info| {
            info.lines()
                .any(|l| l.split(' ').nth(4) == Some(target.as_str()) && l.contains(" - fuse"))
        },
    )
}

/// Clear a mount a crashed agent left (`Transport endpoint is not
/// connected`); refuse one that still answers.
///
/// # Errors
///
/// [`MountError::Busy`]: a live mount is there.
pub fn clear_stale(mountpoint: &Path) -> Result<(), MountError> {
    if !is_mounted(mountpoint) {
        return Ok(());
    }
    match std::fs::read_dir(mountpoint) {
        Err(e) if e.raw_os_error() == Some(libc::ENOTCONN) => {
            tracing::info!(mount = %mountpoint.display(), "drive: clearing a stale mount");
            lazy_unmount(mountpoint);
            Ok(())
        }
        _ => Err(MountError::Busy(mountpoint.display().to_string())),
    }
}

/// `fusermount3 -u -z` (Linux) or `umount -f` (macOS).
pub fn lazy_unmount(mountpoint: &Path) {
    let done = if cfg!(target_os = "macos") {
        Command::new("umount").arg("-f").arg(mountpoint).status()
    } else {
        Command::new(fusermount())
            .args(["-u", "-z"])
            .arg(mountpoint)
            .status()
    };
    if !done.is_ok_and(|s| s.success()) {
        tracing::warn!(mount = %mountpoint.display(), "drive: the unmount did not run");
    }
}

/// The setuid `fusermount3`: on `PATH`, else where NixOS and others keep it.
#[must_use]
pub fn fusermount() -> PathBuf {
    let on_path = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .flat_map(|d| [d.join("fusermount3"), d.join("fusermount")])
            .find(|p| p.is_file())
    });
    on_path
        .or_else(|| {
            [
                "/run/wrappers/bin/fusermount3",
                "/usr/bin/fusermount3",
                "/bin/fusermount3",
                "/usr/bin/fusermount",
            ]
            .iter()
            .map(PathBuf::from)
            .find(|p| p.is_file())
        })
        .unwrap_or_else(|| PathBuf::from("fusermount3"))
}

/// `~/Architect`: the default mount point.
#[must_use]
pub fn default_mount() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Architect"))
}
