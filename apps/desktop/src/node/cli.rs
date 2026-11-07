//! The agent's commands beside `serve`:
//!
//! ```text
//! architect-desktop files pin <path>      # pin the root <path> is in (a path in the drive)
//! architect-desktop files unpin <path>
//! architect-desktop files status [<path>] # the drive, or one root
//! architect-desktop drive-guard <mount>   # started by the drive itself: unmounts when the agent ends
//! ```
//!
//! They find the agent through its config: `--config <file>`,
//! `$ARCHITECT_CONFIG`, else [`default_config`].

use std::ffi::OsString;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use architect_files::agent::files::{Files, FilesHttpClient};

use super::Config;

/// The device agent's config: `$XDG_CONFIG_HOME/architect/agent.toml`
/// (`~/.config/…`), `~/Library/Application Support/Architect/agent.toml`
/// on a Mac.
#[must_use]
pub fn default_config() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Application Support/Architect/agent.toml"));
    }
    let config =
        std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
    Some(config.join("architect").join("agent.toml"))
}

/// The guard's command (the drive starts it; see `architect_files::drive`).
pub const GUARD: &str = "drive-guard";

/// `drive-guard <mount>`.
#[must_use]
pub fn guard(mut args: impl Iterator<Item = OsString>) -> ExitCode {
    let Some(mount) = args.next() else {
        return ExitCode::FAILURE;
    };
    #[cfg(unix)]
    {
        architect_files::drive::guard_main(Path::new(&mount))
    }
    #[cfg(not(unix))]
    {
        let _ = mount;
        ExitCode::FAILURE
    }
}

fn say(text: &str) {
    let _ = writeln!(std::io::stdout(), "{text}");
}

fn fail(text: &str) -> ExitCode {
    let _ = writeln!(std::io::stderr(), "{text}");
    ExitCode::FAILURE
}

/// The agent's config, from `--config`, `$ARCHITECT_CONFIG` or the default.
fn config_from(args: &mut Vec<OsString>) -> Result<Config, String> {
    let mut file = std::env::var_os("ARCHITECT_CONFIG").map(PathBuf::from);
    if let Some(i) = args.iter().position(|a| a == "--config") {
        args.remove(i);
        if i < args.len() {
            file = Some(PathBuf::from(args.remove(i)));
        }
    }
    let file = file
        .or_else(default_config)
        .ok_or("no config: pass --config")?;
    Config::load(Some(&file), |k| std::env::var(k).ok()).map_err(|e| e.to_string())
}

/// The root place a path in the drive is in: `<org>/<section>/<name>`.
#[must_use]
pub fn place_in(drive: &Path, path: &Path) -> Option<String> {
    let abs = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().ok()?.join(path)
    };
    let rel = abs.strip_prefix(drive).ok()?;
    let segs: Vec<String> = rel
        .components()
        .take(3)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    (segs.len() == 3).then(|| segs.join("/"))
}

fn agent_url(config: &Config) -> String {
    let bind = config.bind.replace("0.0.0.0", "127.0.0.1");
    format!("http://{bind}")
}

/// `files pin|unpin|status [<path>]`.
#[must_use]
pub fn files(args: impl Iterator<Item = OsString>) -> ExitCode {
    let mut args: Vec<OsString> = args.collect();
    let config = match config_from(&mut args) {
        Ok(c) => c,
        Err(e) => return fail(&format!("files: {e}")),
    };
    let mut args = args.into_iter();
    let verb = args
        .next()
        .and_then(|v| v.into_string().ok())
        .unwrap_or_default();
    let path = args.next().map(PathBuf::from);
    let Some(drive) = config.drive.clone() else {
        return fail("files: the agent has no drive (`drive` in its config)");
    };
    let place = path.as_deref().and_then(|p| place_in(&drive, p));
    let client = FilesHttpClient::at(agent_url(&config));
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return fail(&e.to_string()),
    };
    let answer: Result<String, String> = rt.block_on(async move {
        let e = |e: architect_files::agent::AgentError| e.to_string();
        match (verb.as_str(), place) {
            ("pin", Some(root)) => Files::pin(&client, root.clone(), None)
                .await
                .map(|at| format!("pinned {root} at {at}"))
                .map_err(e),
            ("unpin", Some(root)) => Files::unpin(&client, root.clone())
                .await
                .map(|()| format!("unpinned {root}"))
                .map_err(e),
            ("status", _) => Files::drive(&client)
                .await
                .map(|d| {
                    format!(
                        "drive: {} ({})\nsyncing: {}\noffline: {}\ncache: {} bytes",
                        d.mount.unwrap_or_default(),
                        if d.mounted { "mounted" } else { "not mounted" },
                        d.syncing,
                        d.offline,
                        d.cache_bytes
                    )
                })
                .map_err(e),
            ("pin" | "unpin", None) => {
                Err(format!("give a path in the drive ({})", drive.display()))
            }
            _ => Err(
                "usage: architect-desktop files pin|unpin|status [<path>] [--config <file>]"
                    .to_owned(),
            ),
        }
    });
    match answer {
        Ok(text) => {
            say(&text);
            ExitCode::SUCCESS
        }
        Err(why) => fail(&format!("files: {why}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_in_the_drive_names_its_root() {
        let drive = Path::new("/home/a/Architect");
        assert_eq!(
            place_in(
                drive,
                Path::new("/home/a/Architect/codywright/Projects/Right Here/Audio Files/x.wav")
            )
            .as_deref(),
            Some("codywright/Projects/Right Here")
        );
        assert_eq!(
            place_in(drive, Path::new("/home/a/Architect/codywright")),
            None
        );
        assert_eq!(place_in(drive, Path::new("/tmp/x")), None);
    }
}
