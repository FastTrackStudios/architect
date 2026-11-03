//! What an agent reads about a running app in one call: `APP_STATUS`.
//!
//! herdr's `status` answers "what is on screen, and what is broken?" for
//! an agent that is about to change the app — the layout, the focus, the
//! overlays and every file or mod error, in one read. Here the answer is
//! assembled from two registries every list carries, so a feature plugs
//! in without the app keeping a list of its sources:
//!
//! - [`StatusBoard`] ([`ActionList::status`]): named sections, each a
//!   function giving its part as JSON (the workspace host's windows and
//!   focus, the theme store's choice and revision). A section is read
//!   when `APP_STATUS` runs, so it is never stale; one that is gone
//!   ([`StatusBoard::remove`]) is no longer asked.
//! - [`Problems`] ([`ActionList::problems`]): what is wrong, each with
//!   its source, the file and field when it has them, and the message. A
//!   source is either pulled ([`Problems::source`]: a function asked at
//!   each read — a theme store's `problems()`) or pushed
//!   ([`Problems::report`]: an extension that failed to load reports, and
//!   clears its report when it loads again).
//!
//! [`register`] puts `APP_STATUS` on a list. Its answer is one object:
//! every section by its name, `problems` (every source's, in source
//! order) and `confirmations` (calls waiting for a person's yes,
//! [`ActionList::pending_confirmations`]).

use std::collections::BTreeMap;
use std::sync::{Arc, PoisonError, RwLock};

use facet::Facet;

use super::{ActionCaller, ActionError, ActionList, Effect, RawJson, check_json};
use crate::action::ActionMeta;

/// `APP_STATUS`'s id.
pub const APP_STATUS: &str = "APP_STATUS";

/// One thing wrong with the app, as an agent is told it.
#[derive(Clone, Debug, PartialEq, Eq, Facet)]
pub struct Problem {
    /// Where it comes from: `theme`, `extension`, `workspace`, `keymap`.
    pub source: String,
    /// The file it is in, when it is in one.
    #[facet(default)]
    pub file: Option<String>,
    /// The field (a manifest key, a token name, a setting), when the
    /// source knows it.
    #[facet(default)]
    pub field: Option<String>,
    pub message: String,
}

impl Problem {
    /// A problem from `source`, saying `message`.
    pub fn new(source: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            file: None,
            field: None,
            message: message.into(),
        }
    }

    /// In `file`.
    #[must_use]
    pub fn file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// About `field`.
    #[must_use]
    pub fn field(mut self, field: impl Into<String>) -> Self {
        self.field = Some(field.into());
        self
    }
}

/// A pulled source: asked at every read.
type Source = Arc<dyn Fn() -> Vec<Problem> + Send + Sync>;

#[derive(Default)]
struct ProblemsInner {
    pulled: BTreeMap<String, Source>,
    pushed: BTreeMap<String, Vec<Problem>>,
}

/// Every source of problems an app has. Cheap to clone: every clone is
/// the same registry, so one can be handed to a part of the app that has
/// a list of its own (an extension host's).
#[derive(Clone, Default)]
pub struct Problems {
    inner: Arc<RwLock<ProblemsInner>>,
}

impl std::fmt::Debug for Problems {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
        f.debug_struct("Problems")
            .field("pulled", &inner.pulled.keys().collect::<Vec<_>>())
            .field("pushed", &inner.pushed.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl Problems {
    /// A source asked at every read, by `key` (a second one by the same
    /// key replaces the first).
    pub fn source(
        &self,
        key: impl Into<String>,
        read: impl Fn() -> Vec<Problem> + Send + Sync + 'static,
    ) {
        self.inner
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .pulled
            .insert(key.into(), Arc::new(read));
    }

    /// What `key` reports now, in place of what it reported before; an
    /// empty report clears it.
    pub fn report(&self, key: impl Into<String>, problems: Vec<Problem>) {
        let key = key.into();
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        if problems.is_empty() {
            inner.pushed.remove(&key);
        } else {
            inner.pushed.insert(key, problems);
        }
    }

    /// Clears `key`'s report and takes its source away.
    pub fn remove(&self, key: &str) {
        let mut inner = self.inner.write().unwrap_or_else(PoisonError::into_inner);
        inner.pulled.remove(key);
        inner.pushed.remove(key);
    }

    /// Every problem there is now: the pulled sources asked (a panic in
    /// one is its own problem), then the reports, each in key order.
    #[must_use]
    pub fn all(&self) -> Vec<Problem> {
        let (pulled, pushed): (Vec<(String, Source)>, Vec<Problem>) = {
            let inner = self.inner.read().unwrap_or_else(PoisonError::into_inner);
            (
                inner
                    .pulled
                    .iter()
                    .map(|(k, s)| (k.clone(), Arc::clone(s)))
                    .collect(),
                inner.pushed.values().flatten().cloned().collect(),
            )
        };
        let mut out = Vec::new();
        for (key, read) in pulled {
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read())) {
                Ok(found) => out.extend(found),
                Err(_) => out.push(Problem::new(key, "its problem source panicked")),
            }
        }
        out.extend(pushed);
        out
    }
}

/// A section: its part of the status, as JSON text.
type Section = Arc<dyn Fn() -> String + Send + Sync>;

/// The named parts of `APP_STATUS`. Cheap to clone.
#[derive(Clone, Default)]
pub struct StatusBoard {
    sections: Arc<RwLock<BTreeMap<String, Section>>>,
}

impl std::fmt::Debug for StatusBoard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.names()).finish()
    }
}

impl StatusBoard {
    /// Section `name`: what `read` gives (JSON text), asked at every read.
    /// A second section by the same name replaces the first. `problems`
    /// and `confirmations` are `APP_STATUS`'s own: a section by either
    /// name is not shown.
    pub fn section(
        &self,
        name: impl Into<String>,
        read: impl Fn() -> String + Send + Sync + 'static,
    ) {
        self.sections
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.into(), Arc::new(read));
    }

    /// Takes section `name` away (its owner is gone).
    pub fn remove(&self, name: &str) {
        self.sections
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }

    /// The sections' names, in order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.sections
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    /// Every section read now, by name: its JSON, or an `{"error": …}`
    /// when it gave something that is not JSON or panicked.
    #[must_use]
    pub fn read(&self) -> Vec<(String, String)> {
        let sections: Vec<(String, Section)> = self
            .sections
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .iter()
            .map(|(k, s)| (k.clone(), Arc::clone(s)))
            .collect();
        sections
            .into_iter()
            .filter(|(name, _)| !RESERVED.contains(&name.as_str()))
            .map(|(name, read)| {
                let json = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| read())) {
                    Ok(json) => match check_json(json.as_bytes()) {
                        Ok(()) => json,
                        Err(why) => {
                            error_json(&format!("it gave something that is not JSON: {why}"))
                        }
                    },
                    Err(_) => error_json("it panicked"),
                };
                (name, json)
            })
            .collect()
    }
}

/// The keys `APP_STATUS` answers itself.
const RESERVED: &[&str] = &["problems", "confirmations"];

fn error_json(message: &str) -> String {
    #[derive(Facet)]
    struct Error {
        error: String,
    }
    facet_json::to_string(&Error {
        error: message.to_owned(),
    })
    .unwrap_or_else(|_| "{}".to_owned())
}

/// A call waiting for a person, as `APP_STATUS` lists it.
#[derive(Clone, Debug, Facet)]
struct Waiting {
    ticket: String,
    id: String,
    caller: String,
}

/// The whole status, as JSON: every section, then `problems` and
/// `confirmations`.
#[must_use]
pub fn status_json(list: &ActionList) -> String {
    let mut out = String::from("{");
    for (name, json) in list.status().read() {
        let key = facet_json::to_string(&name).unwrap_or_else(|_| "\"?\"".to_owned());
        out.push_str(&key);
        out.push(':');
        out.push_str(&json);
        out.push(',');
    }
    let problems =
        facet_json::to_string(&list.problems().all()).unwrap_or_else(|_| "[]".to_owned());
    let waiting: Vec<Waiting> = list
        .pending_confirmations()
        .into_iter()
        .map(|c| Waiting {
            ticket: c.ticket,
            id: c.id,
            caller: c.caller,
        })
        .collect();
    let waiting = facet_json::to_string(&waiting).unwrap_or_else(|_| "[]".to_owned());
    out.push_str("\"problems\":");
    out.push_str(&problems);
    out.push_str(",\"confirmations\":");
    out.push_str(&waiting);
    out.push('}');
    out
}

static STATUS_META: ActionMeta = ActionMeta {
    id: APP_STATUS,
    trait_name: "",
    method_name: "",
    display_name: "App status",
    description: "One read of what is on screen and what is wrong: every status section the app's parts give \
                  (the workspace's windows, focused pane, layout revision and open sheets; the theme's choice, id \
                  and revision), every problem from every source (theme files, extension loads, layouts, keymap: \
                  each with source, file, field and message) and the calls waiting for a person's yes. Read it \
                  before and after changing the app.",
    category: "App",
    group: "Agent tools",
    shortcut: "",
    toggleable: false,
    undo: false,
};

/// Puts `APP_STATUS` on `list`.
///
/// # Errors
///
/// It is there already ([`ActionError::Duplicate`]).
pub fn register(list: &ActionList) -> Result<(), ActionError> {
    let me = list.clone();
    list.register_with(
        &STATUS_META,
        Effect::ReadOnly,
        move |_: &ActionCaller, (): ()| Ok::<_, String>(RawJson::from(status_json(&me))),
    )
}

#[cfg(test)]
mod tests {
    use super::{APP_STATUS, Problem, register};
    use crate::action::ActionList;

    #[test]
    fn status_is_every_section_and_every_problem() {
        let list = ActionList::new();
        register(&list).unwrap();
        list.status()
            .section("theme", || r#"{"id":"night","revision":3}"#.to_owned());
        list.status().section("broken", || "not json".to_owned());
        list.status().section("problems", || "{}".to_owned());
        list.problems().source("theme", || {
            vec![
                Problem::new("theme", "unknown field `colour`")
                    .file("themes/x.json")
                    .field("colour"),
            ]
        });
        list.problems().report(
            "extension:a",
            vec![Problem::new("extension", "bad manifest")],
        );
        let out = String::from_utf8(list.run_json(APP_STATUS, b"").unwrap()).unwrap();
        assert!(out.starts_with(r#"{"broken":{"error":"#), "{out}");
        assert!(
            out.contains(r#""theme":{"id":"night","revision":3}"#),
            "{out}"
        );
        assert!(out.contains(r#""file":"themes/x.json""#), "{out}");
        assert!(out.contains(r#""field":"colour""#), "{out}");
        assert!(out.contains("bad manifest"), "{out}");
        assert!(out.ends_with(r#""confirmations":[]}"#), "{out}");
        assert_eq!(out.matches("\"problems\"").count(), 1, "{out}");
        // A report cleared, a section gone.
        list.problems().report("extension:a", Vec::new());
        list.status().remove("theme");
        let out = String::from_utf8(list.run_json(APP_STATUS, b"").unwrap()).unwrap();
        assert!(
            !out.contains("bad manifest") && !out.contains("night"),
            "{out}"
        );
    }
}
