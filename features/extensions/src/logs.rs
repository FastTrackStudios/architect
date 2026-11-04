//! An extension's recent log lines and errors (#239 X5): what
//! `EXT_LOGS` answers.
//!
//! The host keeps the last [`KEEP`] lines of each extension, from every
//! way it runs: a process's stdout and stderr, a Luau script's `print`
//! and the error it stopped with, a hook that failed, a host-service call
//! that was refused, and what it logs through the host service itself.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;

/// How many lines are kept per extension.
pub const KEEP: usize = 500;

/// Milliseconds since the epoch.
#[must_use]
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// How bad a line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    /// `info`, `warn`, `error`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    /// From `info`, `warn`/`warning`, `error`; info otherwise.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        match text {
            "warn" | "warning" => Self::Warn,
            "error" => Self::Error,
            _ => Self::Info,
        }
    }
}

/// One line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LogLine {
    /// When, in ms since the epoch.
    pub at_ms: u64,
    pub level: Level,
    /// Where it came from: `stdout`, `stderr`, `print`, `script`, `hook`,
    /// `host` (a refused or failed host-service call), `log` (its own,
    /// through the host service), `load`.
    pub source: &'static str,
    pub text: String,
}

/// Every extension's recent lines. Cheap to clone.
#[derive(Clone, Default)]
pub struct Logs {
    inner: Arc<Mutex<BTreeMap<String, VecDeque<LogLine>>>>,
}

impl std::fmt::Debug for Logs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Logs").finish_non_exhaustive()
    }
}

impl Logs {
    /// Keeps a line for `id`.
    pub fn push(&self, id: &str, level: Level, source: &'static str, text: impl Into<String>) {
        let line = LogLine {
            at_ms: now_ms(),
            level,
            source,
            text: text.into(),
        };
        let mut all = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let lines = all.entry(id.to_owned()).or_default();
        if lines.len() == KEEP {
            lines.pop_front();
        }
        lines.push_back(line);
        drop(all);
    }

    /// `id`'s lines at or after `since_ms` (all of them when `None`), the
    /// last `limit`, oldest first.
    // The lines are copied out under the lock, then cut.
    #[allow(clippy::significant_drop_tightening)]
    #[must_use]
    pub fn since(&self, id: &str, since_ms: Option<u64>, limit: usize) -> Vec<LogLine> {
        let kept: Vec<LogLine> = {
            let all = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
            let Some(lines) = all.get(id) else {
                return Vec::new();
            };
            lines
                .iter()
                .filter(|l| since_ms.is_none_or(|s| l.at_ms >= s))
                .cloned()
                .collect()
        };
        let skip = kept.len().saturating_sub(limit);
        kept.into_iter().skip(skip).collect()
    }

    /// The ids that have lines.
    #[must_use]
    pub fn ids(&self) -> Vec<String> {
        self.inner
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{KEEP, Level, Logs};

    #[test]
    fn the_last_lines_are_kept_and_read_from_a_time() {
        let logs = Logs::default();
        for i in 0..(KEEP + 10) {
            logs.push("a", Level::Info, "print", format!("{i}"));
        }
        logs.push("a", Level::Error, "script", "boom");
        let all = logs.since("a", None, usize::MAX);
        assert_eq!(all.len(), KEEP);
        assert_eq!(all.last().unwrap().text, "boom");
        assert_eq!(logs.since("a", None, 2).len(), 2);
        let later = logs.since("a", Some(u64::MAX), 10);
        assert!(later.is_empty());
        assert!(logs.since("b", None, 10).is_empty());
    }
}
