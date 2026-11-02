//! Opening at login: what each platform keeps to start the app, written
//! only for an installed copy, and started with `--background` so a login
//! launch stays out of the way (no window, no Dock icon).
//!
//! - macOS: a `LaunchAgent`, `~/Library/LaunchAgents/<id>.plist` (Task's:
//!   `task:apps/desktop/src/menu_bar.rs`). A `LaunchAgent` rather than
//!   `SMAppService` because it can pass `--background`.
//! - Linux: an XDG autostart entry, `$XDG_CONFIG_HOME/autostart/<id>.desktop`.
//! - Windows: a value under `HKCU\…\CurrentVersion\Run`.
//!
//! The file says where the app is, so it is written again when the app
//! has moved ([`plan`]). Writing it is feature `native` ([`apply`]).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use crate::model::BACKGROUND_ARG;

/// What starts the app at login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginItem {
    /// The bundle identifier (`app.fasttrackstudio.task`): the agent's
    /// label, the file's name.
    pub id: String,
    /// The app's name, as the person reads it.
    pub name: String,
    /// The executable to start.
    pub exe: PathBuf,
}

impl LoginItem {
    /// The arguments it is started with.
    #[must_use]
    pub fn args(&self) -> Vec<String> {
        vec![BACKGROUND_ARG.to_owned()]
    }

    /// The `LaunchAgent`: start `exe --background` at login, in the person's
    /// own (Aqua) session.
    #[must_use]
    pub fn launch_agent(&self) -> String {
        let args = std::iter::once(self.exe.to_string_lossy().into_owned())
            .chain(self.args())
            .fold(String::new(), |mut out, a| {
                let _ = writeln!(out, "        <string>{}</string>", xml(&a));
                out
            });
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{args}    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>LimitLoadToSessionType</key>
    <string>Aqua</string>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
            label = xml(&self.id),
        )
    }

    /// The XDG autostart entry.
    #[must_use]
    pub fn autostart_entry(&self) -> String {
        let mut exec = desktop_quote(&self.exe.to_string_lossy());
        for a in self.args() {
            exec.push(' ');
            exec.push_str(&desktop_quote(&a));
        }
        format!(
            "[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n",
            name = self.name.replace('\n', " "),
        )
    }

    /// The Run key's value: the command line.
    #[must_use]
    pub fn run_value(&self) -> String {
        let mut line = format!("\"{}\"", self.exe.to_string_lossy());
        for a in self.args() {
            line.push(' ');
            line.push_str(&a);
        }
        line
    }
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// An `Exec=` argument, quoted when it has to be (the spec's rules: the
/// reserved characters in double quotes, `"`, `` ` ``, `$` and `\`
/// escaped, `%` doubled).
fn desktop_quote(arg: &str) -> String {
    let arg = arg.replace('%', "%%");
    let plain = arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-_./+=:,@".contains(c));
    if plain && !arg.is_empty() {
        return arg;
    }
    let mut out = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// The platforms whose login items differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    MacOs,
    Linux,
    Windows,
}

impl Os {
    /// The one this is built for; `None` where there is no login item
    /// (a phone, a browser).
    #[must_use]
    pub const fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::MacOs)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else if cfg!(target_os = "windows") {
            Some(Self::Windows)
        } else {
            None
        }
    }
}

/// Why a login item is not written: the app is not an installed copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotInstalled(pub String);

impl std::fmt::Display for NotInstalled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What starts at login for the executable `exe`, if it is an installed
/// copy: a build in `target/` or an app on a mounted disk image must not
/// become what starts at login.
///
/// - macOS: a bundle (`….app/Contents/MacOS/…`) in `/Applications` or
///   `~/Applications`.
/// - Linux: an `AppImage` (`appimage`, its path: what `$APPIMAGE` says), or
///   a binary under `/usr`, `/opt`, `~/.local` or a Nix profile.
/// - Windows: under Program Files or `%LOCALAPPDATA%\Programs`.
///
/// `home` is the person's home; `local_app_data` Windows' `%LOCALAPPDATA%`.
pub fn installed(
    os: Os,
    exe: &Path,
    home: &Path,
    appimage: Option<&Path>,
    local_app_data: Option<&Path>,
) -> Result<PathBuf, NotInstalled> {
    let s = exe.to_string_lossy();
    match os {
        Os::MacOs => {
            let in_bundle = s.contains(".app/Contents/MacOS/");
            let in_apps =
                exe.starts_with("/Applications") || exe.starts_with(home.join("Applications"));
            if in_bundle && in_apps {
                Ok(exe.to_path_buf())
            } else if in_bundle {
                Err(NotInstalled(
                    "move the app to Applications to open it at login".to_owned(),
                ))
            } else {
                Err(NotInstalled(
                    "only an installed app opens at login (this is a build)".to_owned(),
                ))
            }
        }
        Os::Linux => {
            if let Some(image) = appimage {
                return Ok(image.to_path_buf());
            }
            let roots = [
                PathBuf::from("/usr"),
                PathBuf::from("/opt"),
                PathBuf::from("/nix/store"),
                home.join(".local"),
                home.join(".nix-profile"),
            ];
            if roots.iter().any(|r| exe.starts_with(r)) {
                Ok(exe.to_path_buf())
            } else {
                Err(NotInstalled(
                    "only an installed app opens at login (this is a build)".to_owned(),
                ))
            }
        }
        Os::Windows => {
            let lower = s.to_lowercase();
            let programs = local_app_data.is_some_and(|l| exe.starts_with(l.join("Programs")));
            if lower.contains("\\program files") || programs {
                Ok(exe.to_path_buf())
            } else {
                Err(NotInstalled(
                    "only an installed app opens at login (this is a build)".to_owned(),
                ))
            }
        }
    }
}

/// What to do to the login item's file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Write this (there is none, or it says something else: the app
    /// moved).
    Write(String),
    Remove,
    /// It is as it should be.
    Keep,
}

/// What to do, given whether the app should open at login (`want`: the
/// file it should be, `None` for none) and what the file says now
/// (`have`).
#[must_use]
pub fn plan(want: Option<&str>, have: Option<&str>) -> Plan {
    match (want, have) {
        (Some(w), Some(h)) if w == h => Plan::Keep,
        (Some(w), _) => Plan::Write(w.to_owned()),
        (None, Some(_)) => Plan::Remove,
        (None, None) => Plan::Keep,
    }
}

/// What applying a login item did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    Written(PathBuf),
    Removed(PathBuf),
    Unchanged,
}

/// Makes the login item say `on` for `id` (`name` the app's): written for
/// this executable when on and installed, rewritten when it names another
/// path, removed when off. Feature `native`.
#[cfg(feature = "native")]
pub fn apply(id: &str, name: &str, on: bool) -> Result<Applied, String> {
    let Some(os) = Os::current() else {
        return Err("no login items on this platform".to_owned());
    };
    let home = std::env::var_os(if os == Os::Windows {
        "USERPROFILE"
    } else {
        "HOME"
    })
    .map(PathBuf::from)
    .ok_or("no home folder")?;
    let item = if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
        let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
        let exe =
            installed(os, &exe, &home, appimage.as_deref(), local.as_deref()).map_err(|e| e.0)?;
        Some(LoginItem {
            id: id.to_owned(),
            name: name.to_owned(),
            exe,
        })
    } else {
        None
    };
    match os {
        Os::MacOs => apply_file(
            &home.join(format!("Library/LaunchAgents/{id}.plist")),
            item.as_ref().map(LoginItem::launch_agent).as_deref(),
        ),
        Os::Linux => {
            let config = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(".config"));
            apply_file(
                &config.join(format!("autostart/{id}.desktop")),
                item.as_ref().map(LoginItem::autostart_entry).as_deref(),
            )
        }
        Os::Windows => apply_run_key(id, item.as_ref().map(LoginItem::run_value).as_deref()),
    }
}

/// Whether this executable may open at login: an installed copy.
/// Feature `native`.
#[cfg(feature = "native")]
pub fn available() -> Result<(), String> {
    let os = Os::current().ok_or("no login items on this platform")?;
    let home = std::env::var_os(if os == Os::Windows {
        "USERPROFILE"
    } else {
        "HOME"
    })
    .map(PathBuf::from)
    .ok_or("no home folder")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    installed(os, &exe, &home, appimage.as_deref(), local.as_deref())
        .map(drop)
        .map_err(|e| e.0)
}

#[cfg(feature = "native")]
fn apply_file(path: &Path, want: Option<&str>) -> Result<Applied, String> {
    let have = std::fs::read_to_string(path).ok();
    match plan(want, have.as_deref()) {
        Plan::Keep => Ok(Applied::Unchanged),
        Plan::Write(body) => {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
            }
            std::fs::write(path, body).map_err(|e| e.to_string())?;
            Ok(Applied::Written(path.to_path_buf()))
        }
        Plan::Remove => {
            std::fs::remove_file(path).map_err(|e| e.to_string())?;
            Ok(Applied::Removed(path.to_path_buf()))
        }
    }
}

#[cfg(feature = "native")]
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// The Run key through `reg.exe`: one value, the app's id.
#[cfg(feature = "native")]
fn apply_run_key(id: &str, want: Option<&str>) -> Result<Applied, String> {
    let have = run_key(&["query", RUN_KEY, "/v", id]).ok();
    let had = have.as_deref().and_then(|out| {
        out.lines()
            .find(|l| l.trim_start().starts_with(id))
            .and_then(|l| l.split_once("REG_SZ"))
            .map(|(_, v)| v.trim().to_owned())
    });
    match plan(want, had.as_deref()) {
        Plan::Keep => Ok(Applied::Unchanged),
        Plan::Write(value) => {
            run_key(&["add", RUN_KEY, "/v", id, "/t", "REG_SZ", "/d", &value, "/f"])?;
            Ok(Applied::Written(PathBuf::from(RUN_KEY)))
        }
        Plan::Remove => {
            run_key(&["delete", RUN_KEY, "/v", id, "/f"])?;
            Ok(Applied::Removed(PathBuf::from(RUN_KEY)))
        }
    }
}

#[cfg(feature = "native")]
fn run_key(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("reg")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}
