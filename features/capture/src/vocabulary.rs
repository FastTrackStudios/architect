//! What the composer offers to pick from (#445): the people a capture
//! can be assigned to, the projects it can be filed in, the vault's tags
//! as labels, and the contexts things get done in.
//!
//! Each comes from whichever action knows it, while it is on the list
//! ([`SOURCES`], read by `Capturer::vocabulary`): tags from `TAGS_LIST`,
//! projects and their members from the Projects extension's
//! `PROJECTS_LIST`, contexts and assignees from the Tasks extension's
//! `TASKS_LIST`, and people's notes (a note under a `People` folder) from
//! `VAULT_LIST`. Pure data here, so it is tested without a vault.

use serde_json::Value;

/// The actions read, with their arguments (JSON).
pub const SOURCES: &[(&str, &str)] = &[
    ("TAGS_LIST", "{}"),
    ("PROJECTS_LIST", "{}"),
    ("TASKS_LIST", r#"{"view":"all"}"#),
    ("VAULT_LIST", "{}"),
];

/// Contexts offered before the vault has any (Task's).
pub const DEFAULT_CONTEXTS: &[&str] = &["errands", "chores", "home", "calls", "computer", "out"];

/// What there is to pick from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Vocabulary {
    /// People by name (a person's note's title): an assignee is written
    /// as a link to it.
    pub people: Vec<String>,
    /// Projects by title: filed as `[[Title]]`, as quick add writes it.
    pub projects: Vec<String>,
    /// The vault's tags, without the `#`.
    pub labels: Vec<String>,
    /// Contexts in use, without the `@`.
    pub contexts: Vec<String>,
}

/// Whether `path` is a note (`.md`, any case).
#[must_use]
pub fn is_note(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"))
}

/// `[[Folder/Name]]`, `Folder/Name.md` or `Name` as `Name`.
#[must_use]
pub fn bare_name(link: &str) -> String {
    let s = link.trim().trim_start_matches("[[").trim_end_matches("]]");
    let s = s.split('|').next().unwrap_or(s);
    let s = s.rsplit('/').next().unwrap_or(s);
    s.trim_end_matches(".md").trim().to_owned()
}

fn push(into: &mut Vec<String>, name: &str) {
    let name = name.trim();
    if !name.is_empty() && !into.iter().any(|n| n.eq_ignore_ascii_case(name)) {
        into.push(name.to_owned());
    }
}

fn strings(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(s)) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// The rows of an answer: itself when it is a list, else its list under
/// `key`.
fn rows<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.as_array()
        .or_else(|| v.get(key).and_then(Value::as_array))
        .map_or(&[], Vec::as_slice)
}

impl Vocabulary {
    /// Read from what [`SOURCES`] answered (`(action id, answer)`).
    #[must_use]
    pub fn from_answers(answers: &[(&str, Value)]) -> Self {
        let mut out = Self::default();
        for (id, v) in answers {
            match *id {
                "TAGS_LIST" => {
                    for row in rows(v, "tags") {
                        if let Some(t) = row.get("tag").and_then(Value::as_str).or(row.as_str()) {
                            push(&mut out.labels, t.trim_start_matches('#'));
                        }
                    }
                }
                "PROJECTS_LIST" => {
                    for row in rows(v, "projects") {
                        if let Some(t) = row.get("title").and_then(Value::as_str) {
                            push(&mut out.projects, t);
                        }
                        for m in strings(row.get("members")) {
                            push(&mut out.people, &bare_name(&m));
                        }
                    }
                }
                "TASKS_LIST" => {
                    for row in rows(v, "tasks") {
                        for c in strings(row.get("contexts")) {
                            push(&mut out.contexts, c.trim_start_matches('@'));
                        }
                        for a in strings(row.get("assignees")) {
                            push(&mut out.people, &bare_name(&a));
                        }
                    }
                }
                "VAULT_LIST" => {
                    for row in rows(v, "notes") {
                        let path = row.get("path").and_then(Value::as_str).unwrap_or_default();
                        let person = path.starts_with("People/") || path.contains("/People/");
                        if person && is_note(path) {
                            let title = row
                                .get("title")
                                .and_then(Value::as_str)
                                .map_or_else(|| bare_name(path), str::to_owned);
                            push(&mut out.people, &title);
                        }
                    }
                }
                _ => {}
            }
        }
        out.people.sort_by_key(|p| p.to_lowercase());
        out.labels.sort_by_key(|l| l.to_lowercase());
        out
    }

    /// The contexts to offer: those in use, then Task's defaults.
    #[must_use]
    pub fn context_choices(&self) -> Vec<String> {
        let mut out = self.contexts.clone();
        for c in DEFAULT_CONTEXTS {
            push(&mut out, c);
        }
        out
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn each_action_adds_what_it_knows() {
        let v = Vocabulary::from_answers(&[
            (
                "TAGS_LIST",
                json!([{ "tag": "mixing", "notes": 3 }, { "tag": "gigs" }]),
            ),
            (
                "PROJECTS_LIST",
                json!({ "projects": [{ "title": "Album", "members": ["[[People/Sam Okafor]]"] }] }),
            ),
            (
                "TASKS_LIST",
                json!({ "tasks": [{ "contexts": ["@errands", "@home"], "assignees": ["[[Alex Rivera]]"] }] }),
            ),
            (
                "VAULT_LIST",
                json!({ "notes": [
                    { "path": "Areas/People/Sam Okafor.md", "title": "Sam Okafor" },
                    { "path": "Areas/People/Jo Ng.md" },
                    { "path": "Notes/People to call.md" }
                ] }),
            ),
        ]);
        assert_eq!(v.labels, ["gigs", "mixing"]);
        assert_eq!(v.projects, ["Album"]);
        assert_eq!(v.contexts, ["errands", "home"]);
        assert_eq!(
            v.people,
            ["Alex Rivera", "Jo Ng", "Sam Okafor"],
            "once each"
        );
    }

    #[test]
    fn contexts_offered_lead_with_the_vault_s_and_never_repeat() {
        let v = Vocabulary {
            contexts: vec!["studio".into(), "errands".into()],
            ..Vocabulary::default()
        };
        let c = v.context_choices();
        assert_eq!(c[0], "studio");
        assert_eq!(c.iter().filter(|x| *x == "errands").count(), 1);
        assert!(c.contains(&"chores".to_owned()));
    }

    #[test]
    fn a_link_reads_as_its_name() {
        assert_eq!(bare_name("[[People/Sam Okafor|Sam]]"), "Sam Okafor");
        assert_eq!(bare_name("Areas/People/Jo Ng.md"), "Jo Ng");
    }
}
