//! The device's agent, installed: it starts at login, mounts the drive,
//! and restarts if it dies.
//!
//! - **Linux:** a systemd user unit, `architect-agent.service`. `PATH`
//!   includes the setuid `fusermount3`, and `ExecStopPost` unmounts the
//!   drive however the agent ended.
//! - **macOS:** a launchd agent, `com.fasttrackstudios.architect.agent`
//!   (`RunAtLoad`, `KeepAlive`).
//!
//! Both run `<this binary> serve --config <agent.toml>`. The config's
//! defaults: [`Defaults`].

use std::path::{Path, PathBuf};
use std::process::Command;

/// The unit (Linux) and label (macOS).
pub const UNIT: &str = "architect-agent.service";
pub const LABEL: &str = "com.fasttrackstudios.architect.agent";
/// Where the device agent listens (loopback only): the app finds it here.
pub const BIND: &str = "127.0.0.1:18471";

/// Where a device's agent keeps things, by default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Defaults {
    /// `agent.toml`.
    pub config: PathBuf,
    /// The agent's state: its keys, documents, indexes, the blob cache.
    pub data: PathBuf,
    /// Working copies of pinned roots (inside the data dir).
    pub store: PathBuf,
    /// The drive.
    pub mount: PathBuf,
}

impl Defaults {
    /// For the user whose home is `home`, honouring `$XDG_CONFIG_HOME` and
    /// `$XDG_DATA_HOME` on Linux.
    #[must_use]
    pub fn for_home(home: &Path, xdg_config: Option<PathBuf>, xdg_data: Option<PathBuf>) -> Self {
        let (config_dir, data) = if cfg!(target_os = "macos") {
            let app = home.join("Library/Application Support/Architect");
            (app.clone(), app.join("agent"))
        } else {
            (
                xdg_config
                    .unwrap_or_else(|| home.join(".config"))
                    .join("architect"),
                xdg_data
                    .unwrap_or_else(|| home.join(".local/share"))
                    .join("architect")
                    .join("agent"),
            )
        };
        Self {
            config: config_dir.join("agent.toml"),
            store: data.join("roots"),
            data,
            mount: home.join("Architect"),
        }
    }

    /// For this user.
    #[must_use]
    pub fn here() -> Option<Self> {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        Some(Self::for_home(
            &home,
            std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        ))
    }

    /// A new `agent.toml`: this device's agent with its drive.
    #[must_use]
    pub fn config_text(&self, name: &str) -> String {
        format!(
            "# The device's Architect agent (architect-desktop agent install).\n\
             # docs/content/features/drive.md lists every key.\n\
             name = {name:?}\n\
             bind = {BIND:?}\n\
             data_dir = {data:?}\n\
             # Working copies of pinned projects and libraries.\n\
             files_dir = {store:?}\n\
             # The drive: every root this device knows, as a folder.\n\
             drive = {mount:?}\n",
            data = self.data.display().to_string(),
            store = self.store.display().to_string(),
            mount = self.mount.display().to_string(),
        )
    }
}

/// `key = "value"` set in a TOML file's text (top-level string keys only),
/// replacing the line that sets it or adding one.
#[must_use]
pub fn set_key(text: &str, key: &str, value: &str) -> String {
    let line = format!("{key} = {value:?}");
    let mut found = false;
    let mut out: Vec<String> = text
        .lines()
        .map(|l| {
            let set_here = l
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == key && !l.trim_start().starts_with('#'));
            if set_here && !found {
                found = true;
                line.clone()
            } else {
                l.to_owned()
            }
        })
        .collect();
    if !found {
        out.push(line);
    }
    let mut joined = out.join("\n");
    joined.push('\n');
    joined
}

/// A top-level string key's value in a TOML file's text.
#[must_use]
pub fn get_key(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches('"').to_owned())
}

/// The systemd user unit that runs `exe serve --config <config>`.
#[must_use]
pub fn systemd_unit(exe: &Path, config: &Path, mount: Option<&Path>, fusermount: &Path) -> String {
    let path = format!(
        "{}:/run/wrappers/bin:/usr/local/bin:/usr/bin:/bin",
        exe.parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
    let stop = mount.map_or_else(String::new, |m| {
        format!(
            "ExecStopPost=-{} -u -z {}\n",
            fusermount.display(),
            quote(m)
        )
    });
    format!(
        "[Unit]\n\
         Description=Architect agent: sync, the drive\n\
         After=network-online.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         Environment=PATH={path}\n\
         ExecStart={exe} serve --config {config}\n\
         {stop}\
         Restart=on-failure\n\
         RestartSec=3\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        exe = quote(exe),
        config = quote(config),
    )
}

fn quote(p: &Path) -> String {
    let s = p.display().to_string();
    if s.contains(' ') {
        format!("\"{s}\"")
    } else {
        s
    }
}

/// The launchd agent that runs `exe serve --config <config>`.
#[must_use]
pub fn launchd_plist(exe: &Path, config: &Path, log: &Path) -> String {
    let esc = |p: &Path| {
        p.display()
            .to_string()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\"><dict>\n\
         <key>Label</key><string>{LABEL}</string>\n\
         <key>ProgramArguments</key><array><string>{exe}</string><string>serve</string><string>--config</string><string>{config}</string></array>\n\
         <key>RunAtLoad</key><true/>\n\
         <key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>\n\
         <key>StandardOutPath</key><string>{log}</string>\n\
         <key>StandardErrorPath</key><string>{log}</string>\n\
         </dict></plist>\n",
        exe = esc(exe),
        config = esc(config),
        log = esc(log),
    )
}

/// Where the service file goes: `~/.config/systemd/user` or
/// `~/Library/LaunchAgents`.
#[must_use]
pub fn service_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map_or_else(|| home.join(".config"), PathBuf::from)
            .join("systemd/user")
    })
}

/// What `install` does.
#[derive(Debug, Clone)]
pub struct Install {
    pub exe: PathBuf,
    pub config: PathBuf,
    /// Where the unit or plist goes (default [`service_dir`]).
    pub service_dir: PathBuf,
    /// Enable and start it now.
    pub start: bool,
    pub defaults: Defaults,
    pub name: String,
}

/// Install the agent: write `agent.toml` if there is none, the unit or
/// plist, and (with `start`) enable and start it. Returns the service file.
///
/// # Errors
///
/// A file could not be written, or `systemctl`/`launchctl` failed.
pub fn install(i: &Install) -> Result<PathBuf, String> {
    let io = |e: std::io::Error| e.to_string();
    if let Some(dir) = i.config.parent() {
        std::fs::create_dir_all(dir).map_err(io)?;
    }
    if !i.config.exists() {
        std::fs::write(&i.config, i.defaults.config_text(&i.name)).map_err(io)?;
    }
    let text = std::fs::read_to_string(&i.config).map_err(io)?;
    let mount = get_key(&text, "drive").map(PathBuf::from);
    std::fs::create_dir_all(&i.service_dir).map_err(io)?;
    if cfg!(target_os = "macos") {
        let plist = i.service_dir.join(format!("{LABEL}.plist"));
        let log = i.defaults.data.join("agent.log");
        std::fs::write(&plist, launchd_plist(&i.exe, &i.config, &log)).map_err(io)?;
        if i.start {
            let uid = Command::new("id").arg("-u").output().map_err(io)?;
            let domain = format!("gui/{}", String::from_utf8_lossy(&uid.stdout).trim());
            let _ = Command::new("launchctl")
                .args(["bootout", &domain])
                .arg(&plist)
                .status();
            run(Command::new("launchctl")
                .args(["bootstrap", &domain])
                .arg(&plist))?;
        }
        return Ok(plist);
    }
    let unit = i.service_dir.join(UNIT);
    std::fs::write(
        &unit,
        systemd_unit(&i.exe, &i.config, mount.as_deref(), &super::fusermount()),
    )
    .map_err(io)?;
    if i.start {
        run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
        run(Command::new("systemctl").args(["--user", "enable", "--now", UNIT]))?;
    }
    Ok(unit)
}

/// Stop the agent and remove its service file (its config and data stay).
///
/// # Errors
///
/// The file could not be removed.
pub fn uninstall(service_dir: &Path) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let plist = service_dir.join(format!("{LABEL}.plist"));
        if let Ok(uid) = Command::new("id").arg("-u").output() {
            let domain = format!("gui/{}", String::from_utf8_lossy(&uid.stdout).trim());
            let _ = Command::new("launchctl")
                .args(["bootout", &domain])
                .arg(&plist)
                .status();
        }
        return remove(&plist);
    }
    let _ = Command::new("systemctl")
        .args(["--user", "disable", "--now", UNIT])
        .status();
    remove(&service_dir.join(UNIT))?;
    let _ = Command::new("systemctl")
        .args(["--user", "daemon-reload"])
        .status();
    Ok(())
}

/// Start the installed agent if it is not running (the app joins it).
///
/// # Errors
///
/// `systemctl`/`launchctl` failed.
pub fn start_installed() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let uid = Command::new("id")
            .arg("-u")
            .output()
            .map_err(|e| e.to_string())?;
        let target = format!(
            "gui/{}/{LABEL}",
            String::from_utf8_lossy(&uid.stdout).trim()
        );
        return run(Command::new("launchctl").args(["kickstart", &target]));
    }
    run(Command::new("systemctl").args(["--user", "start", UNIT]))
}

/// Restart the installed agent (a changed config applies).
///
/// # Errors
///
/// `systemctl`/`launchctl` failed.
pub fn restart_installed() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let uid = Command::new("id")
            .arg("-u")
            .output()
            .map_err(|e| e.to_string())?;
        let target = format!(
            "gui/{}/{LABEL}",
            String::from_utf8_lossy(&uid.stdout).trim()
        );
        return run(Command::new("launchctl").args(["kickstart", "-k", &target]));
    }
    run(Command::new("systemctl").args(["--user", "restart", UNIT]))
}

/// Whether the service file is installed in `service_dir`.
#[must_use]
pub fn installed(service_dir: &Path) -> bool {
    if cfg!(target_os = "macos") {
        service_dir.join(format!("{LABEL}.plist")).exists()
    } else {
        service_dir.join(UNIT).exists()
    }
}

fn remove(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_put_the_drive_at_home_and_the_store_in_the_data_dir() {
        let d = Defaults::for_home(Path::new("/home/a"), None, None);
        assert_eq!(d.mount, Path::new("/home/a/Architect"));
        assert!(d.store.starts_with(&d.data));
        let text = d.config_text("laptop");
        assert_eq!(
            get_key(&text, "drive").as_deref(),
            Some("/home/a/Architect")
        );
        assert_eq!(get_key(&text, "bind").as_deref(), Some(BIND));
        let moved = set_key(&text, "drive", "/mnt/work/Architect");
        assert_eq!(
            get_key(&moved, "drive").as_deref(),
            Some("/mnt/work/Architect")
        );
        assert_eq!(moved.matches("drive =").count(), 1);
    }

    #[test]
    fn the_unit_runs_serve_and_unmounts_after() {
        let unit = systemd_unit(
            Path::new("/opt/architect/architect-desktop"),
            Path::new("/home/a/.config/architect/agent.toml"),
            Some(Path::new("/home/a/Architect")),
            Path::new("/run/wrappers/bin/fusermount3"),
        );
        assert!(unit.contains("ExecStart=/opt/architect/architect-desktop serve --config /home/a/.config/architect/agent.toml"));
        assert!(
            unit.contains("ExecStopPost=-/run/wrappers/bin/fusermount3 -u -z /home/a/Architect")
        );
        assert!(unit.contains("WantedBy=default.target"));
        let plist = launchd_plist(
            Path::new("/A/architect"),
            Path::new("/c.toml"),
            Path::new("/l.log"),
        );
        assert!(plist.contains("<key>RunAtLoad</key><true/>"));
    }
}
