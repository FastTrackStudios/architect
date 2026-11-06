//! What a capture can become: its targets.
//!
//! The app's own is a plain note (`note`: filed into a folder, its
//! properties kept). Extensions add theirs with a manifest table, read by
//! the host into its registry (`EXT_CAPTURE_TARGETS`):
//!
//! ```toml
//! [[capture_targets]]
//! id = "task"
//! name = "Task"
//! icon = "check"
//! action = "TASKS_CAPTURE"   # makes one from a capture (the contract below)
//! chips = ["projects", "due", "priority", "tags", "contexts", "recurrence"]
//! tokens = [{ prefix = "key:", key = "key" }]   # quick-add words it adds
//! ```
//!
//! A target's `action` is its half of the contract: it is run with a
//! [`Capture`] as JSON and answers at least `{path}`, the note it wrote.
//! The capture's inbox note goes once it answers.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::parse::TokenRule;

/// The app's own target: a plain note.
pub const NOTE: &str = "note";

/// One target, as the composer and the inbox offer it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub description: String,
    /// What makes one from a capture; empty for the app's own note.
    pub action: String,
    /// The properties it offers as chips, in order.
    pub chips: Vec<String>,
    /// Quick-add words it adds to the parser.
    pub tokens: Vec<TokenRule>,
    /// The extension it is from; `app` for the app's own.
    pub from: String,
}

impl Target {
    /// The app's own plain note.
    #[must_use]
    pub fn note() -> Self {
        Self {
            id: NOTE.to_owned(),
            name: "Note".to_owned(),
            icon: "file-text".to_owned(),
            description: "A plain note, filed out of the inbox with its properties.".to_owned(),
            chips: vec!["projects".to_owned(), "tags".to_owned()],
            from: "app".to_owned(),
            ..Self::default()
        }
    }

    /// The registry's answer (`EXT_CAPTURE_TARGETS`, a JSON array) read,
    /// the app's note first; an id taken already is the first's.
    #[must_use]
    pub fn all_from(registry: &Value) -> Vec<Self> {
        let mut out = vec![Self::note()];
        for t in registry.as_array().into_iter().flatten() {
            if let Ok(t) = serde_json::from_value::<Self>(t.clone())
                && !t.id.is_empty()
                && !t.action.is_empty()
                && !out.iter().any(|o| o.id == t.id)
            {
                out.push(t);
            }
        }
        out
    }
}

/// What a target's action is given.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Capture {
    pub title: String,
    pub body: String,
    /// The parsed and hand-set properties (`TaskNotes`' keys for the
    /// core's tokens).
    pub properties: Map<String, Value>,
    /// Where it was captured from (`ui`, `window`, `mcp`).
    pub source: String,
    /// Its inbox note, when it is processed from the inbox.
    pub inbox: Option<String>,
}

/// Every target's tokens, for the parser.
#[must_use]
pub fn tokens(targets: &[Target]) -> Vec<TokenRule> {
    targets.iter().flat_map(|t| t.tokens.clone()).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_note_comes_first_and_an_id_is_the_first_s() {
        let all = Target::all_from(&json!([
            { "id": "task", "name": "Task", "action": "TASKS_CAPTURE", "from": "tasks",
              "tokens": [{ "prefix": "key:", "key": "key" }] },
            { "id": "note", "name": "Mine", "action": "X_NOTE" },
            { "id": "task", "name": "Another", "action": "Y_CAPTURE" },
            { "id": "bare", "name": "No action" },
        ]));
        let ids: Vec<&str> = all.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, ["note", "task"]);
        assert_eq!(all[1].from, "tasks");
        assert_eq!(tokens(&all)[0].prefix, "key:");
    }
}
