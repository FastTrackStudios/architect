//! The app's links on Linux (#629, #186): `architect://open?url=<a share
//! link>` opens in the app.
//!
//! The AppImage's desktop entry declares the scheme
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
    let write = |path: &Path, text: &str| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    };
    write(&entry, &desktop_entry(name, exe, id))?;
    let mimeapps = config_home.join("mimeapps.list");
    let before = std::fs::read_to_string(&mimeapps).unwrap_or_default();
    write(&mimeapps, &with_default(&before, &scheme_type(), &file))?;
    Ok(Registered { entry, mimeapps })
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
    }
}
