//! A client of an app's control socket (feature `client`): blocking, with
//! no runtime, one request at a time.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::protocol::{Code, Failure, Reply, Request};

#[cfg(unix)]
type Conn = std::os::unix::net::UnixStream;

#[cfg(not(unix))]
type Conn = std::fs::File;

/// A connection to a running app.
#[derive(Debug)]
pub struct Client {
    path: PathBuf,
    reader: BufReader<Conn>,
    writer: Conn,
    next: u64,
}

impl Client {
    /// Connect to the app serving `path`.
    ///
    /// # Errors
    ///
    /// [`Code::Unavailable`]: nothing answers there.
    pub fn connect(path: &Path) -> Result<Self, Failure> {
        let unavailable = |e: std::io::Error| {
            Failure::new(
                Code::Unavailable,
                format!(
                    "no app answers on {}: {e} — is it running, with its control socket on \
                     (FTS_CONTROL=1)?",
                    path.display()
                ),
            )
        };
        let writer = open(path).map_err(unavailable)?;
        let reader = BufReader::new(writer.try_clone().map_err(unavailable)?);
        Ok(Self {
            path: path.to_path_buf(),
            reader,
            writer,
            next: 1,
        })
    }

    /// Where it is connected.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Ask `request`, and wait for its answer.
    ///
    /// # Errors
    ///
    /// [`Code::Unavailable`] when the app went away; [`Code::Protocol`]
    /// when what came back is not an answer to it.
    pub fn call(&mut self, request: &Request) -> Result<Reply, Failure> {
        self.call_raw(request.method().name(), &request.params())
    }

    /// Ask `method` with `params` as they are, unchecked here: what the
    /// app makes of a request it may refuse.
    ///
    /// # Errors
    ///
    /// As [`Self::call`].
    pub fn call_raw(&mut self, method: &str, params: &Value) -> Result<Reply, Failure> {
        let id = self.next;
        self.next = self.next.saturating_add(1);
        let message = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        let gone = |e: std::io::Error| {
            Failure::new(
                Code::Unavailable,
                format!("the app on {} went away: {e}", self.path.display()),
            )
        };
        let mut bytes = message.to_string().into_bytes();
        bytes.push(b'\n');
        self.writer.write_all(&bytes).map_err(gone)?;
        self.writer.flush().map_err(gone)?;
        let mut line = String::new();
        let n = self.reader.read_line(&mut line).map_err(gone)?;
        if n == 0 {
            return Err(Failure::new(
                Code::Unavailable,
                format!("the app on {} closed the connection", self.path.display()),
            ));
        }
        let value: Value = serde_json::from_str(&line)
            .map_err(|e| Failure::new(Code::Protocol, format!("not JSON: {e}")))?;
        let (answered, reply) = Reply::from_line(&value)?;
        if answered != id && !answered.is_null() {
            return Err(Failure::new(
                Code::Protocol,
                format!("asked {id}, answered {answered}"),
            ));
        }
        Ok(reply)
    }
}

#[cfg(unix)]
fn open(path: &Path) -> std::io::Result<Conn> {
    std::os::unix::net::UnixStream::connect(path)
}

/// A named pipe's client end is a file opened for reading and writing.
#[cfg(not(unix))]
fn open(path: &Path) -> std::io::Result<Conn> {
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
}
