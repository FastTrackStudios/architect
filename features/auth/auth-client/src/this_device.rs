//! Who this device is, the same answer from every app on it.
//!
//! The issuer's device list (`DeviceService`) is keyed by a device id the
//! device chooses. Two apps on one iPad — Task and Studio — must choose
//! the same one, or the account shows the iPad twice. They cannot share a
//! file (each iOS app has its own container), so the id is derived from
//! what the platform already gives every app of one developer alike:
//!
//! - **iOS**: `identifierForVendor`, the same for every app of the team on
//!   the device (it changes only once every one of them is deleted);
//! - **macOS**: the host UUID (`gethostuuid`);
//! - **Linux**: `/etc/machine-id`.
//!
//! The raw id never leaves the device: [`install_id`] is a v5 UUID of it,
//! and [`device_id`] another of that and the account, so an install
//! signed in as somebody else is a different device to the issuer (which
//! refuses an id another account holds).
//!
//! `FTS_INSTALL_ID` overrides the install id: a scratch device in a test,
//! or a second install on one machine that should count as its own.

use uuid::Uuid;

/// The namespace an install id is made in.
const INSTALL_NAMESPACE: Uuid = Uuid::from_u128(0x6f1c_2d3e_8a47_4b9e_9c55_0d1e_7a2b_f3c4);

/// The environment variable that overrides [`install_id`].
pub const INSTALL_ID_ENV: &str = "FTS_INSTALL_ID";

/// This machine's install id: the same in every app on it, `None` where
/// the platform gives nothing to derive it from.
#[must_use]
pub fn install_id() -> Option<Uuid> {
    if let Some(id) = std::env::var(INSTALL_ID_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<Uuid>().ok())
    {
        return Some(id);
    }
    machine_id().map(|raw| install_from(&raw))
}

/// The install id made from a platform's raw machine id.
#[must_use]
pub fn install_from(raw: &str) -> Uuid {
    Uuid::new_v5(
        &INSTALL_NAMESPACE,
        raw.trim().to_ascii_lowercase().as_bytes(),
    )
}

/// The id `install` registers under for `account` (the issuer's subject).
#[must_use]
pub fn device_id(install: Uuid, account: &str) -> Uuid {
    Uuid::new_v5(&install, account.as_bytes())
}

/// What a person calls this device: the iPad's name, the Mac's computer
/// name, the host name.
#[must_use]
pub fn name() -> String {
    platform_name()
        .map(|n| n.trim().trim_end_matches(".local").to_owned())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| platform().to_owned())
}

/// `ios`, `macos`, `linux`, `windows`, `android`, …
#[must_use]
pub const fn platform() -> &'static str {
    std::env::consts::OS
}

#[cfg(target_os = "ios")]
fn ui_device() -> objc2::rc::Retained<objc2_ui_kit::UIDevice> {
    // SAFETY: only read-only properties of `currentDevice` are asked for
    // (its vendor id and name), which UIKit answers from any thread; the
    // sync driver that asks runs on its own runtime, never on main.
    let mtm = unsafe { objc2::MainThreadMarker::new_unchecked() };
    objc2_ui_kit::UIDevice::currentDevice(mtm)
}

#[cfg(target_os = "ios")]
fn machine_id() -> Option<String> {
    let id = ui_device().identifierForVendor()?;
    Some(id.UUIDString().to_string())
}

#[cfg(target_os = "ios")]
fn platform_name() -> Option<String> {
    Some(ui_device().name().to_string())
}

#[cfg(target_os = "macos")]
fn machine_id() -> Option<String> {
    let mut id = [0u8; 16];
    let wait = libc::timespec {
        tv_sec: 1,
        tv_nsec: 0,
    };
    // SAFETY: `id` is the 16 bytes `gethostuuid` writes; `wait` lives
    // across the call.
    let rc = unsafe { libc::gethostuuid(id.as_mut_ptr(), &raw const wait) };
    (rc == 0).then(|| Uuid::from_bytes(id).to_string())
}

#[cfg(target_os = "macos")]
fn platform_name() -> Option<String> {
    let out = std::process::Command::new("/usr/sbin/scutil")
        .args(["--get", "ComputerName"])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    String::from_utf8(out.stdout).ok()
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn machine_id() -> Option<String> {
    ["/etc/machine-id", "/var/lib/dbus/machine-id"]
        .iter()
        .find_map(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
}

#[cfg(not(any(target_os = "ios", target_os = "macos")))]
fn platform_name() -> Option<String> {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
}

#[cfg(not(any(
    target_os = "ios",
    target_os = "macos",
    target_os = "linux",
    target_os = "android"
)))]
fn machine_id() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_app_on_a_machine_derives_the_same_device() {
        let a = install_from("6F1C2D3E-8A47-4B9E-9C55-0D1E7A2BF3C4");
        let b = install_from(" 6f1c2d3e-8a47-4b9e-9c55-0d1e7a2bf3c4\n");
        assert_eq!(a, b, "case and whitespace do not make a second device");
        assert_eq!(device_id(a, "acct"), device_id(b, "acct"));
        assert_ne!(device_id(a, "acct"), device_id(a, "other"));
        assert_ne!(a, install_from("another machine"));
    }

    #[test]
    fn this_machine_has_an_install_id_and_a_name() {
        if cfg!(target_os = "macos") {
            assert!(install_id().is_some(), "a Mac always has a host uuid");
        }
        assert_eq!(install_id(), install_id(), "stable across calls");
        assert!(!name().is_empty());
    }
}
