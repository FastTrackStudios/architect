//! The app's links on Linux (#629, #186): `architect://open?url=<a share
//! link>` opens in the app.
//!
//! The `AppImage`'s desktop entry declares the scheme
//! (`scripts/desktop/linux.sh`); a build run from a folder (a dev build,
//! Nix) registers itself with `architect-desktop links register`: a
//! desktop entry for this binary under `$XDG_DATA_HOME/applications`
//! that handles `x-scheme-handler/architect`, made the default in
//! `$XDG_CONFIG_HOME/mimeapps.list`. The OS then starts the binary with
//! the link as its argument; a second launch hands it to the running one
//! (`architect_presence::single_instance`), and the window opens it in
//! its Shared tab (`architect_app::guest::incoming`).

use std::path::{Path, PathBuf};

use architect_spaces::guest::APP_SCHEME;

/// The MIME type the app's scheme is.
#[must_use]
pub fn scheme_type() -> String {
    format!("x-scheme-handler/{APP_SCHEME}")
}

/// The desktop entry that opens the app's links with `exe`.
#[must_use]
pub fn desktop_entry(name: &str, exe: &Path, class: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={name}\nExec={} %u\nTerminal=false\nNoDisplay=true\nMimeType={};\nStartupWMClass={class}\n",
        quote_exec(&exe.to_string_lossy()),
        scheme_type()
    )
}

/// An `Exec` argument quoted as the Desktop Entry spec reads it.
fn quote_exec(arg: &str) -> String {
    if arg
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c))
    {
        return arg.to_owned();
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

/// `mimeapps.list`'s text with `handler` the default for `mime`, the
/// rest kept as it was.
#[must_use]
pub fn with_default(list: &str, mime: &str, handler: &str) -> String {
    const SECTION: &str = "[Default Applications]";
    let line = format!("{mime}={handler}");
    let mut out = Vec::new();
    let mut in_section = false;
    let mut done = false;
    for l in list.lines() {
        let trimmed = l.trim();
        if trimmed.starts_with('[') {
            if in_section && !done {
                out.push(line.clone());
                done = true;
            }
            in_section = trimmed == SECTION;
        } else if in_section
            && trimmed
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == mime)
        {
            if !done {
                out.push(line.clone());
                done = true;
            }
            continue;
        }
        out.push(l.to_owned());
    }
    if !done {
        if !in_section {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(SECTION.to_owned());
        }
        out.push(line);
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Where a registration was written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Registered {
    pub entry: PathBuf,
    pub mimeapps: PathBuf,
}

/// Register `exe` (the app `name`, its window class and desktop file
/// `id`) as the handler of the app's links: its desktop entry in
/// `data_home/applications`, the default in `config_home/mimeapps.list`.
///
/// # Errors
///
/// A file could not be written.
pub fn register(
    name: &str,
    id: &str,
    exe: &Path,
    data_home: &Path,
    config_home: &Path,
) -> Result<Registered, String> {
    let file = format!("{id}.links.desktop");
    let entry = data_home.join("applications").join(&file);
    let write = |path: &Path, text: &str| replace(path, text);
    write(&entry, &desktop_entry(name, exe, id))?;
    let mimeapps = config_home.join("mimeapps.list");
    let before = std::fs::read_to_string(&mimeapps).unwrap_or_default();
    write(&mimeapps, &with_default(&before, &scheme_type(), &file))?;
    Ok(Registered { entry, mimeapps })
}

/// Write `text` to `path` whole or not at all: to a temporary file beside
/// it, then renamed over it, so a crash or a full disk never leaves half a
/// `mimeapps.list` (which every other app's defaults live in too). A
/// symlinked file (a dotfiles manager's) is written where it points, and
/// the link kept. The file keeps its mode (a private `0600` list stays
/// private); a new one gets the umask's.
fn replace(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    let at = |p: &Path, e: std::io::Error| format!("{}: {e}", p.display());
    let target = if path.is_symlink() {
        std::fs::canonicalize(path).map_err(|e| at(path, e))?
    } else {
        path.to_path_buf()
    };
    let dir = target
        .parent()
        .ok_or_else(|| format!("{}: no folder", target.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| at(dir, e))?;
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temp = dir.join(format!(".{name}.{}.tmp", std::process::id()));
    let kept = std::fs::metadata(&target).ok().map(|m| m.permissions());
    let written = std::fs::File::create(&temp).and_then(|mut f| {
        if let Some(mode) = kept {
            f.set_permissions(mode)?;
        }
        f.write_all(text.as_bytes())?;
        f.sync_all()
    });
    if let Err(e) = written.and_then(|()| std::fs::rename(&temp, &target)) {
        let _ = std::fs::remove_file(&temp);
        return Err(at(&target, e));
    }
    Ok(())
}

/// `$XDG_DATA_HOME` and `$XDG_CONFIG_HOME`, or their defaults under
/// `$HOME`.
#[must_use]
pub fn xdg_homes() -> Option<(PathBuf, PathBuf)> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let var = |name: &str, under: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home.as_ref().map(|h| h.join(under)))
    };
    Some((
        var("XDG_DATA_HOME", ".local/share")?,
        var("XDG_CONFIG_HOME", ".config")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_hands_the_link_to_the_binary() {
        let entry = desktop_entry(
            "Architect",
            Path::new("/opt/My Apps/architect-desktop"),
            "app.fasttrackstudio.architect",
        );
        assert!(
            entry.contains("Exec=\"/opt/My Apps/architect-desktop\" %u\n"),
            "{entry}"
        );
        assert!(entry.contains("MimeType=x-scheme-handler/architect;\n"));
        assert!(entry.contains("NoDisplay=true"));
    }

    #[test]
    fn the_default_is_set_and_the_rest_kept() {
        let mime = "x-scheme-handler/architect";
        assert_eq!(
            with_default("", mime, "a.desktop"),
            "[Default Applications]\nx-scheme-handler/architect=a.desktop\n"
        );
        let before = "[Default Applications]\ntext/html=firefox.desktop\nx-scheme-handler/architect=old.desktop\n\n[Added Associations]\ntext/plain=gedit.desktop\n";
        let after = with_default(before, mime, "a.desktop");
        assert!(after.contains("text/html=firefox.desktop"));
        assert!(after.contains("x-scheme-handler/architect=a.desktop"));
        assert!(!after.contains("old.desktop"));
        assert!(after.contains("[Added Associations]\ntext/plain=gedit.desktop"));
        let other = "[Added Associations]\ntext/plain=gedit.desktop\n";
        let after = with_default(other, mime, "a.desktop");
        assert!(
            after.ends_with("[Default Applications]\nx-scheme-handler/architect=a.desktop\n"),
            "{after}"
        );
    }

    #[test]
    fn register_writes_the_entry_and_the_default() {
        let tmp = tempfile::tempdir().unwrap();
        let (data, config) = (tmp.path().join("data"), tmp.path().join("config"));
        let done = register(
            "Architect",
            "app.fasttrackstudio.architect",
            Path::new("/usr/bin/architect-desktop"),
            &data,
            &config,
        )
        .unwrap();
        assert_eq!(
            done.entry,
            data.join("applications/app.fasttrackstudio.architect.links.desktop")
        );
        let list = std::fs::read_to_string(done.mimeapps).unwrap();
        assert!(
            list.contains("x-scheme-handler/architect=app.fasttrackstudio.architect.links.desktop")
        );
        // Twice is the same as once.
        register(
            "Architect",
            "app.fasttrackstudio.architect",
            Path::new("/usr/bin/architect-desktop"),
            &data,
            &config,
        )
        .unwrap();
        let again = std::fs::read_to_string(config.join("mimeapps.list")).unwrap();
        assert_eq!(again, list);
        // Written whole, by a rename: no temporary file is left beside it.
        let left: Vec<_> = std::fs::read_dir(&config)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(left, ["mimeapps.list"], "{left:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_list_is_written_where_it_points_and_stays_a_link() {
        let tmp = tempfile::tempdir().unwrap();
        let (data, config, dotfiles) = (
            tmp.path().join("data"),
            tmp.path().join("config"),
            tmp.path().join("dotfiles"),
        );
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&dotfiles).unwrap();
        let real = dotfiles.join("mimeapps.list");
        std::fs::write(&real, "[Default Applications]\ntext/html=firefox.desktop\n").unwrap();
        std::os::unix::fs::symlink(&real, config.join("mimeapps.list")).unwrap();
        register(
            "Architect",
            "app.fasttrackstudio.architect",
            Path::new("/usr/bin/architect-desktop"),
            &data,
            &config,
        )
        .unwrap();
        assert!(config.join("mimeapps.list").is_symlink());
        let list = std::fs::read_to_string(&real).unwrap();
        assert!(list.contains("text/html=firefox.desktop"), "{list}");
        assert!(list.contains("x-scheme-handler/architect="), "{list}");
    }

    #[cfg(unix)]
    #[test]
    fn a_rewritten_list_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt as _;
        let tmp = tempfile::tempdir().unwrap();
        let (data, config) = (tmp.path().join("data"), tmp.path().join("config"));
        std::fs::create_dir_all(&config).unwrap();
        let list = config.join("mimeapps.list");
        std::fs::write(&list, "[Default Applications]\ntext/html=firefox.desktop\n").unwrap();
        std::fs::set_permissions(&list, std::fs::Permissions::from_mode(0o600)).unwrap();
        register(
            "Architect",
            "app.fasttrackstudio.architect",
            Path::new("/usr/bin/architect-desktop"),
            &data,
            &config,
        )
        .unwrap();
        let mode = std::fs::metadata(&list).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o600, "{mode:o}");
        let text = std::fs::read_to_string(&list).unwrap();
        assert!(text.contains("x-scheme-handler/architect="), "{text}");
    }
}
