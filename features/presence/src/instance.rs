//! One instance: a second launch hands its arguments (and any URLs) to the
//! running one, which comes forward, and ends. A background app is easy
//! to launch twice: from the Dock, from a terminal, at login.
//!
//! The hand-off is one JSON line on a socket in the person's own runtime
//! folder, answered `ok`. On macOS, opening the bundle again is also
//! handled by Launch Services, which activates the running copy (a Dock
//! click, [`Event::Reopen`](crate::lifecycle::Event::Reopen)); the socket
//! covers a launch of the binary itself. Feature `native` binds it
//! ([`claim`]); Windows has no socket here and says so.

use serde::{Deserialize, Serialize};

/// What a second launch hands over.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Handoff {
    /// Its arguments, the program's name left out.
    #[serde(default)]
    pub args: Vec<String>,
    /// Where it was started from.
    #[serde(default)]
    pub cwd: Option<String>,
    /// URLs it was asked to open (`task://…`, a file's).
    #[serde(default)]
    pub urls: Vec<String>,
}

impl Handoff {
    /// This process's: its arguments, its folder; the arguments that look
    /// like URLs (`scheme://…`) as its URLs too.
    #[must_use]
    pub fn this_process() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let urls = args.iter().filter(|a| is_url(a)).cloned().collect();
        Self {
            args,
            cwd: std::env::current_dir()
                .ok()
                .map(|d| d.to_string_lossy().into_owned()),
            urls,
        }
    }

    /// One line of JSON (no newline in it).
    #[must_use]
    pub fn encode(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_owned())
    }

    /// A line [`Self::encode`] wrote.
    pub fn decode(line: &str) -> Result<Self, String> {
        serde_json::from_str(line.trim()).map_err(|e| e.to_string())
    }

    /// Whether it was started to stay in the background (a login launch
    /// that found the app running): it asks nothing to come forward.
    #[must_use]
    pub fn background(&self) -> bool {
        self.args.iter().any(|a| a == crate::model::BACKGROUND_ARG)
    }
}

fn is_url(arg: &str) -> bool {
    arg.split_once("://").is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+-.".contains(c))
    })
}

/// The socket's file name for the app `id`.
#[must_use]
pub fn socket_name(id: &str) -> String {
    let safe: String = id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("{safe}.instance")
}

/// The answer the running instance gives a hand-off.
pub const ACK: &str = "ok";

#[cfg(all(feature = "native", unix))]
pub use unix::{Claim, claim, claim_at, runtime_dir};

#[cfg(all(feature = "native", unix))]
mod unix {
    use std::io::{BufRead as _, BufReader, Write as _};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    use super::{ACK, Handoff, socket_name};

    /// How long a hand-off waits for the running instance's answer.
    const WAIT: Duration = Duration::from_secs(3);

    /// What claiming the app's one instance found.
    pub enum Claim {
        /// This is the one: second launches' hand-offs arrive on `handoffs`.
        First {
            handoffs: futures_channel::mpsc::UnboundedReceiver<Handoff>,
        },
        /// Another is running and took the hand-off: end this one.
        Running,
    }

    /// The person's own folder for sockets: `$XDG_RUNTIME_DIR`, else the
    /// temporary folder (per person on macOS).
    #[must_use]
    pub fn runtime_dir() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(std::env::temp_dir)
    }

    /// Claims app `id`'s one instance, handing `handoff` to the running
    /// one if there is one.
    pub fn claim(id: &str, handoff: &Handoff) -> std::io::Result<Claim> {
        claim_at(&runtime_dir().join(socket_name(id)), handoff)
    }

    /// [`claim`] on the socket at `path`.
    pub fn claim_at(path: &Path, handoff: &Handoff) -> std::io::Result<Claim> {
        if send(path, handoff).is_ok() {
            return Ok(Claim::Running);
        }
        // Nothing answered: a socket left by a copy that crashed.
        let _ = std::fs::remove_file(path);
        let listener = match UnixListener::bind(path) {
            Ok(l) => l,
            // Two launches at once: the other bound it first.
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
                send(path, handoff)?;
                return Ok(Claim::Running);
            }
            Err(e) => return Err(e),
        };
        let (tx, handoffs) = futures_channel::mpsc::unbounded();
        std::thread::Builder::new()
            .name("presence-instance".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    if let Some(h) = receive(stream) {
                        tracing::info!(
                            presence.handoff.args = h.args.len(),
                            presence.handoff.urls = h.urls.len(),
                            "presence: a second launch handed over"
                        );
                        if tx.unbounded_send(h).is_err() {
                            return;
                        }
                    }
                }
            })?;
        Ok(Claim::First { handoffs })
    }

    fn send(path: &Path, handoff: &Handoff) -> std::io::Result<()> {
        let mut stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(WAIT))?;
        stream.set_write_timeout(Some(WAIT))?;
        stream.write_all(handoff.encode().as_bytes())?;
        stream.write_all(b"\n")?;
        let mut answer = String::new();
        BufReader::new(stream).read_line(&mut answer)?;
        if answer.trim() == ACK {
            Ok(())
        } else {
            Err(std::io::Error::other("the running instance did not answer"))
        }
    }

    fn receive(mut stream: UnixStream) -> Option<Handoff> {
        stream.set_read_timeout(Some(WAIT)).ok()?;
        let mut line = String::new();
        BufReader::new(&stream).read_line(&mut line).ok()?;
        let handoff = Handoff::decode(&line).ok()?;
        stream.write_all(format!("{ACK}\n").as_bytes()).ok()?;
        Some(handoff)
    }
}

#[cfg(all(feature = "native", not(unix)))]
pub use other::{Claim, claim};

/// Where there is no instance socket (Windows): every launch is the first,
/// and [`claim`] says why.
#[cfg(all(feature = "native", not(unix)))]
mod other {
    use super::Handoff;

    pub enum Claim {
        First {
            handoffs: futures_channel::mpsc::UnboundedReceiver<Handoff>,
        },
        Running,
    }

    /// Fails: one instance is not built for this platform yet (a named
    /// pipe on Windows).
    pub fn claim(id: &str, handoff: &Handoff) -> std::io::Result<Claim> {
        let _ = (id, handoff);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "one instance is not built for this platform yet: every launch runs",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{Handoff, is_url, socket_name};

    #[test]
    fn a_handoff_is_one_line_both_ways() {
        let h = Handoff {
            args: vec!["--open".into(), "task://inbox".into()],
            cwd: Some("/tmp".into()),
            urls: vec!["task://inbox".into()],
        };
        let line = h.encode();
        assert!(!line.contains('\n'));
        assert_eq!(Handoff::decode(&format!("{line}\n")), Ok(h));
        assert_eq!(Handoff::decode("{}"), Ok(Handoff::default()));
        assert!(Handoff::decode("nope").is_err());
    }

    #[test]
    fn urls_and_socket_names() {
        assert!(is_url("task://inbox/3"));
        assert!(is_url("file:///Users/me/a.md"));
        assert!(!is_url("--background"));
        assert!(!is_url("://x"));
        assert_eq!(
            socket_name("app.fasttrackstudio.task"),
            "app.fasttrackstudio.task.instance"
        );
        assert_eq!(socket_name("a/b c"), "a_b_c.instance");
    }
}
