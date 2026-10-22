//! Where a served Action List is, for a client to find without asking.
//!
//! An app serving its list on loopback ([`crate::http`]) says so in two
//! places:
//!
//! - **The discovery file** `<config>/fts/<name>-mcp.json` ([`publish`]):
//!   `{"url": "...", "token": "..." | null}`, readable by this user only
//!   (mode `0600` on Unix) because the token, when there is one, is in it.
//!   Any client on the machine reads it ([`read`]) to find the port the app
//!   bound and the token it wants.
//! - **A project's `.mcp.json`** ([`project_mcp_json`]): the file Claude
//!   Code reads from a repository, naming the server by URL. It carries no
//!   token — it is committed.
//!
//! `<config>` is the platform config directory (`~/.config` on Linux,
//! `~/Library/Application Support` on macOS); the `*_in` variants take
//! the directory instead, for tests and for an app that keeps its own.

use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

/// The variable that turns an app's MCP server off: `FTS_MCP=0`.
pub const DISABLE_ENV: &str = "FTS_MCP";

/// The directory under the platform config directory that discovery
/// files live in.
pub const DISCOVERY_DIR: &str = "fts";

/// What a discovery file says: where the server is and the token it
/// wants, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Discovery {
    /// `http://127.0.0.1:<port>/mcp`.
    pub url: String,
    /// The bearer token a client must send, when the server wants one.
    pub token: Option<String>,
}

/// Whether `FTS_MCP=0` asks for no MCP server.
#[must_use]
pub fn disabled() -> bool {
    disabled_by(std::env::var(DISABLE_ENV).ok().as_deref())
}

/// Whether a value of `FTS_MCP` turns the server off: only `0` does.
pub(crate) fn disabled_by(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim() == "0")
}

/// `<config>/fts/<name>-mcp.json`, where [`publish`] writes `name`'s
/// discovery file.
///
/// # Errors
///
/// `name` is not a plain file-name part, or this platform has no config
/// directory.
pub fn discovery_path(name: &str) -> io::Result<PathBuf> {
    discovery_path_in(&config_dir()?, name)
}

/// `<config_dir>/fts/<name>-mcp.json`.
///
/// # Errors
///
/// `name` is empty or not a plain file-name part (`/`, `\`, a leading
/// `.`, a control character).
pub fn discovery_path_in(config_dir: &Path, name: &str) -> io::Result<PathBuf> {
    check_name(name)?;
    Ok(config_dir
        .join(DISCOVERY_DIR)
        .join(format!("{name}-mcp.json")))
}

/// Say where `name`'s server is: write `url` and `token` to
/// `<config>/fts/<name>-mcp.json`, mode `0600`, replacing what was there.
/// Returns the file's path.
///
/// # Errors
///
/// See [`publish_in`]; also when this platform has no config directory.
pub fn publish(name: &str, url: &str, token: Option<&str>) -> io::Result<PathBuf> {
    publish_in(&config_dir()?, name, url, token)
}

/// [`publish`] under `config_dir` instead of the platform's.
///
/// The file is written whole or not at all (a temporary file created
/// `0600`, synced, renamed over the old one), so a client never reads half
/// of it and the token is never readable by anyone else, not even briefly.
///
/// # Errors
///
/// `name` is not a plain file-name part, or the directory or file could
/// not be written.
pub fn publish_in(
    config_dir: &Path,
    name: &str,
    url: &str,
    token: Option<&str>,
) -> io::Result<PathBuf> {
    let path = discovery_path_in(config_dir, name)?;
    let text = json!({ "url": url, "token": token }).to_string();
    write_private(&path, text.as_bytes())?;
    Ok(path)
}

/// What `name`'s discovery file says.
///
/// # Errors
///
/// No file (`NotFound`), or one that is not a discovery file
/// (`InvalidData`).
pub fn read(name: &str) -> io::Result<Discovery> {
    read_in(&config_dir()?, name)
}

/// [`read`] under `config_dir`.
///
/// # Errors
///
/// As [`read`].
pub fn read_in(config_dir: &Path, name: &str) -> io::Result<Discovery> {
    let text = fs::read_to_string(discovery_path_in(config_dir, name)?)?;
    let value: Value = serde_json::from_str(&text).map_err(invalid_data)?;
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_data("a discovery file without a url"))?
        .to_owned();
    let token = match value.get("token") {
        None | Some(Value::Null) => None,
        Some(Value::String(t)) => Some(t.clone()),
        Some(_) => return Err(invalid_data("a token that is not a string")),
    };
    Ok(Discovery { url, token })
}

/// Take `name`'s discovery file away, when the server stops. Nothing to
/// take away is not an error.
///
/// # Errors
///
/// The file is there and could not be removed.
pub fn unpublish(name: &str) -> io::Result<()> {
    unpublish_in(&config_dir()?, name)
}

/// [`unpublish`] under `config_dir`.
///
/// # Errors
///
/// As [`unpublish`].
pub fn unpublish_in(config_dir: &Path, name: &str) -> io::Result<()> {
    match fs::remove_file(discovery_path_in(config_dir, name)?) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// Name `url` as the MCP server `name` in `dir/.mcp.json` — the file
/// Claude Code reads from a project — as `{"type": "http", "url": url}`
/// under `mcpServers`, keeping every other server and key in the file.
/// Creates the file when there is none. Returns whether it changed: an
/// entry already saying the same is left alone, file untouched.
///
/// No token goes in: the file is committed. A server that wants one is
/// found through its discovery file instead.
///
/// Keys are written sorted (serde_json's map), so a hand-ordered file may
/// be reordered the first time this changes it.
///
/// # Errors
///
/// The file is not a JSON object (or its `mcpServers` is not one) —
/// refused rather than overwritten — or it could not be read or written.
pub fn project_mcp_json(dir: &Path, name: &str, url: &str) -> io::Result<bool> {
    if name.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an MCP server needs a name",
        ));
    }
    let path = dir.join(".mcp.json");
    let mut root = match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str::<Value>(&text).map_err(invalid_data)? {
            Value::Object(map) => map,
            _ => return Err(invalid_data(".mcp.json is not a JSON object")),
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Map::new(),
        Err(e) => return Err(e),
    };
    let servers = root
        .entry("mcpServers")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| invalid_data(".mcp.json's mcpServers is not a JSON object"))?;
    let entry = json!({ "type": "http", "url": url });
    if servers.get(name) == Some(&entry) {
        return Ok(false);
    }
    servers.insert(name.to_owned(), entry);
    let mut text = serde_json::to_string_pretty(&Value::Object(root)).map_err(invalid_data)?;
    text.push('\n');
    fs::write(&path, text)?;
    Ok(true)
}

/// The platform config directory.
fn config_dir() -> io::Result<PathBuf> {
    dirs::config_dir().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "this platform has no config directory",
        )
    })
}

/// A server name is one plain file-name part: it becomes
/// `<name>-mcp.json`.
fn check_name(name: &str) -> io::Result<()> {
    let plain = !name.is_empty()
        && !name.starts_with('.')
        && !name
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control());
    if plain {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name:?} is not a plain file name for a discovery file"),
        ))
    }
}

/// Write `bytes` to `path` whole, readable by this user only: a fresh
/// `0600` temporary file beside it, synced, renamed over it.
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no directory"))?;
    fs::create_dir_all(dir)?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("mcp.json");
    let temp = dir.join(format!(".{file_name}.{}.tmp", std::process::id()));
    let _ = fs::remove_file(&temp);
    let written = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

fn invalid_data(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A fresh directory under the system temp dir, removed on drop.
    pub(crate) struct Scratch(pub PathBuf);

    impl Scratch {
        pub(crate) fn new(tag: &str) -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "architect-mcp-{tag}-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch dir");
            Self(dir)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn publish_writes_url_and_token_for_this_user_only() {
        let scratch = Scratch::new("publish");
        let path = publish_in(&scratch.0, "session", "http://127.0.0.1:1/mcp", Some("t"))
            .expect("published");
        assert_eq!(path, scratch.0.join("fts").join("session-mcp.json"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).expect("there").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
        }
        assert_eq!(
            read_in(&scratch.0, "session").expect("read"),
            Discovery {
                url: "http://127.0.0.1:1/mcp".into(),
                token: Some("t".into())
            }
        );
        // Republished without a token: replaced, `null`, still 0600.
        publish_in(&scratch.0, "session", "http://127.0.0.1:2/mcp", None).expect("again");
        let text = fs::read_to_string(&path).expect("text");
        assert!(text.contains(r#""token":null"#), "{text}");
        assert_eq!(read_in(&scratch.0, "session").expect("read").token, None);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).expect("there").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "mode {mode:o}");
        }
        // No temporary left beside it.
        let left: Vec<_> = fs::read_dir(scratch.0.join("fts"))
            .expect("dir")
            .map(|e| e.expect("entry").file_name())
            .collect();
        assert_eq!(left, vec![std::ffi::OsString::from("session-mcp.json")]);
        unpublish_in(&scratch.0, "session").expect("unpublished");
        assert!(!path.exists());
        unpublish_in(&scratch.0, "session").expect("nothing to remove is fine");
    }

    #[test]
    fn a_name_is_one_plain_file_name_part() {
        let dir = Path::new("/c");
        for bad in ["", ".hidden", "../x", "a/b", "a\\b", "a:b", "a\nb"] {
            assert!(discovery_path_in(dir, bad).is_err(), "{bad:?}");
        }
        assert!(discovery_path_in(dir, "signal-engine").is_ok());
    }

    #[test]
    fn only_zero_turns_it_off() {
        assert!(disabled_by(Some("0")));
        assert!(disabled_by(Some(" 0 ")));
        assert!(!disabled_by(None));
        assert!(!disabled_by(Some("1")));
        assert!(!disabled_by(Some("")));
    }

    #[test]
    fn project_mcp_json_adds_and_keeps_the_rest() {
        let scratch = Scratch::new("project");
        let file = scratch.0.join(".mcp.json");
        assert!(
            project_mcp_json(&scratch.0, "session", "http://127.0.0.1:47811/mcp").expect("new")
        );
        let first: Value =
            serde_json::from_str(&fs::read_to_string(&file).expect("text")).expect("json");
        assert_eq!(
            first,
            json!({"mcpServers": {"session": {"type": "http", "url": "http://127.0.0.1:47811/mcp"}}})
        );
        // Same again: untouched.
        assert!(
            !project_mcp_json(&scratch.0, "session", "http://127.0.0.1:47811/mcp").expect("same")
        );
        // Another server and a foreign key stay.
        fs::write(
            &file,
            r#"{"other":1,"mcpServers":{"git":{"command":"git-mcp"},"session":{"type":"http","url":"old"}}}"#,
        )
        .expect("seeded");
        assert!(project_mcp_json(&scratch.0, "session", "http://127.0.0.1:1/mcp").expect("moved"));
        let text = fs::read_to_string(&file).expect("text");
        assert!(text.ends_with('\n'));
        let after: Value = serde_json::from_str(&text).expect("json");
        assert_eq!(
            after,
            json!({"other": 1, "mcpServers": {
                "git": {"command": "git-mcp"},
                "session": {"type": "http", "url": "http://127.0.0.1:1/mcp"}
            }})
        );
        // Not an object: refused, left as it was.
        fs::write(&file, "[1]").expect("seeded");
        assert_eq!(
            project_mcp_json(&scratch.0, "session", "u")
                .expect_err("refused")
                .kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(fs::read_to_string(&file).expect("text"), "[1]");
    }
}
