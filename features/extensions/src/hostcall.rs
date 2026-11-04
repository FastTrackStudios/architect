//! The host service's calls, as data (#178): what an extension asks the
//! app for, the same over every way it runs.
//!
//! A [`HostRequest`] is one call; its answer is a [`HostAnswer`]. Both
//! are plain JSON, so each way an extension runs carries them as they
//! are — a process over vox (`ExtensionHost::call`, `guest::host`), a
//! Luau script through its `host` global, an in-process extension through
//! its `linked::Handle`, and the WASM runner (#167) through one imported
//! function taking and giving JSON text. The host side, which checks each
//! call against what the extension may reach, is `crate::hostapi`
//! (feature `host`).
//!
//! | call | what it asks | needs |
//! |---|---|---|
//! | `run {action, args?}` | runs an app action, as the extension (for whoever it acts for) | its own namespace, or `capabilities.actions` and `.effect` |
//! | `org` | the current organization | `reads = ["org"]` |
//! | `selection` | what is selected and focused | `reads = ["selection"]` |
//! | `open {path}` | opens a document or file in the app | `reads = ["documents"]` |
//! | `read_document {path}` | a document's text | `reads = ["documents"]` |
//! | `read_file {path}` / `write_file {path, text}` | a file: relative paths are in its own data folder | its own folder, or `capabilities.folders` |
//! | `setting {key}` / `set_setting {key, value}` | one of its own settings (`<id>.…`) | nothing |
//! | `data {key}` / `set_data {key, value?}` | its own data, kept for it | nothing |
//! | `log {text, level?}` | a line in its log (`EXT_LOGS`) | nothing |

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One call an extension makes to its host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostRequest {
    Run {
        action: String,
        #[serde(default)]
        args: Value,
        /// The ticket a person approved, when the first run answered
        /// [`HostError::ConfirmationRequired`].
        #[serde(default)]
        ticket: Option<String>,
    },
    Org,
    Selection,
    Open {
        path: String,
    },
    ReadDocument {
        path: String,
    },
    ReadFile {
        path: String,
    },
    WriteFile {
        path: String,
        text: String,
    },
    Setting {
        key: String,
    },
    SetSetting {
        key: String,
        value: Value,
    },
    Data {
        key: String,
    },
    SetData {
        key: String,
        #[serde(default)]
        value: Option<Value>,
    },
    Log {
        text: String,
        #[serde(default)]
        level: Option<String>,
    },
}

impl HostRequest {
    /// Its name (`run`, `read_file`): the `op` it is sent as.
    #[must_use]
    pub const fn op(&self) -> &'static str {
        match self {
            Self::Run { .. } => "run",
            Self::Org => "org",
            Self::Selection => "selection",
            Self::Open { .. } => "open",
            Self::ReadDocument { .. } => "read_document",
            Self::ReadFile { .. } => "read_file",
            Self::WriteFile { .. } => "write_file",
            Self::Setting { .. } => "setting",
            Self::SetSetting { .. } => "set_setting",
            Self::Data { .. } => "data",
            Self::SetData { .. } => "set_data",
            Self::Log { .. } => "log",
        }
    }
}

/// Why the host did not do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostError {
    /// Outside what it may reach: the capability it would need
    /// (`actions`, `effect`, `reads.selection`, `folders`, `settings`)
    /// and why, in words. Granting it is the person's
    /// (`EXT_GRANT`, the extensions page).
    Refused { capability: String, why: String },
    /// A person must say yes to this exact call first (an extension acting
    /// for an agent ran something destructive): run it again with the
    /// ticket once they have.
    ConfirmationRequired { action: String, ticket: String },
    /// The extension is not loaded (unloaded, or replaced) any more.
    NotLoaded { id: String },
    /// Nothing of that name (an action, a document, a file).
    NotFound { what: String },
    /// It was allowed and did not work.
    Failed { message: String },
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused { capability, why } => write!(f, "refused ({capability}): {why}"),
            Self::ConfirmationRequired { action, ticket } => {
                write!(
                    f,
                    "{action}: a person must confirm it first (ticket {ticket})"
                )
            }
            Self::NotLoaded { id } => write!(f, "{id} is not loaded"),
            Self::NotFound { what } => write!(f, "no {what}"),
            Self::Failed { message } => f.write_str(message),
        }
    }
}

impl std::error::Error for HostError {}

/// A call's answer: `{"ok": value}` or `{"error": {kind, …}}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostAnswer {
    Ok(Value),
    Error(HostError),
}

impl HostAnswer {
    /// The answer as a `Result`.
    ///
    /// # Errors
    ///
    /// The host refused or failed.
    pub fn into_result(self) -> Result<Value, HostError> {
        match self {
            Self::Ok(v) => Ok(v),
            Self::Error(e) => Err(e),
        }
    }

    /// As JSON text (what crosses a wire).
    #[must_use]
    pub fn to_json(&self) -> String {
        serde_json::to_string(self)
            .unwrap_or_else(|e| format!(r#"{{"error":{{"kind":"failed","message":"{e}"}}}}"#))
    }

    /// From JSON text.
    #[must_use]
    pub fn from_json(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_else(|e| {
            Self::Error(HostError::Failed {
                message: format!("the host's answer is not one: {e}"),
            })
        })
    }
}

impl From<Result<Value, HostError>> for HostAnswer {
    fn from(r: Result<Value, HostError>) -> Self {
        match r {
            Ok(v) => Self::Ok(v),
            Err(e) => Self::Error(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{HostAnswer, HostError, HostRequest};

    #[test]
    fn calls_and_answers_are_plain_json() {
        let r: HostRequest = serde_json::from_str(r#"{"op":"run","action":"APP_STATUS"}"#).unwrap();
        assert_eq!(r.op(), "run");
        let r: HostRequest = serde_json::from_str(r#"{"op":"selection"}"#).unwrap();
        assert_eq!(r, HostRequest::Selection);
        let refused = HostAnswer::Error(HostError::Refused {
            capability: "actions".to_owned(),
            why: "no".to_owned(),
        });
        let text = refused.to_json();
        assert!(text.contains(r#""kind":"refused""#), "{text}");
        assert_eq!(HostAnswer::from_json(&text), refused);
        assert_eq!(
            HostAnswer::from_json(r#"{"ok":3}"#).into_result(),
            Ok(serde_json::json!(3))
        );
    }
}
