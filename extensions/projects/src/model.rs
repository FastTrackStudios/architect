//! Projects, goals and milestones, as the notes they are. Pure: no host,
//! no files.
//!
//! - A **project** is architect's built-in `project` kind (#344): a folder
//!   whose `project.md` is the project, its properties the kind's
//!   (`state`, `start`, `due`, `up`, `members`, `icon`, `tags`) plus its
//!   type's (`projectType: software` and what that type adds).
//! - A **goal** (`type: goal`) is Task's `Goal`
//!   (`task:features/goal/goal-proto/src/model.rs`): a `kind` (lifetime …
//!   weekly), a `status` (aspiration … abandoned), a `parent` goal, a
//!   `targetDate`, and here a `project`.
//! - A **milestone** (`type: milestone`) is Task's `Milestone`
//!   (`task:features/milestone/milestone-proto/src/model.rs`): a
//!   `project`, a `goal`, `status` open or closed, a `dueDate`. A task
//!   names its milestone in `milestone`, so a milestone's progress is its
//!   tasks'.
//!
//! Where Task points by UUID a note points by link
//! (`project: "[[Projects/Site/project|Site]]"`).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// What a project's note is called inside its folder.
pub const PROJECT_PAGE: &str = "project.md";

/// The key a project's type is written under.
pub const TYPE_KEY: &str = "projectType";

/// A project's state: the `project` kind's `state` options.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Planning,
    Active,
    OnHold,
    Done,
}

impl State {
    pub const ALL: [Self; 4] = [Self::Active, Self::Planning, Self::OnHold, Self::Done];

    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Active => "active",
            Self::OnHold => "on_hold",
            Self::Done => "done",
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Planning => "Planning",
            Self::Active => "Active",
            Self::OnHold => "On hold",
            Self::Done => "Done",
        }
    }

    /// The kind's states and their synonyms (`paused`, `completed`, …);
    /// an unknown one reads as planning.
    #[must_use]
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "active" | "in_progress" | "doing" | "started" | "open" => Self::Active,
            "on_hold" | "hold" | "paused" | "waiting" | "blocked" => Self::OnHold,
            "done" | "completed" | "complete" | "shipped" | "archived" | "cancelled"
            | "canceled" | "abandoned" | "closed" => Self::Done,
            _ => Self::Planning,
        }
    }
}

/// One project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    /// Its `project.md`'s vault path.
    pub path: String,
    /// Its folder: what is in it is the project's.
    pub folder: String,
    pub title: String,
    /// Its type's id (`software`), when it has one.
    pub project_type: Option<String>,
    /// As written (`state`, else `status`: the demo vault's older key).
    pub state: String,
    pub start: Option<String>,
    pub due: Option<String>,
    /// The area or project it sits under.
    pub up: Option<String>,
    pub members: Vec<String>,
    pub icon: Option<String>,
    pub tags: Vec<String>,
}

impl Project {
    #[must_use]
    pub fn from_values(path: &str, values: &Map<String, Value>) -> Self {
        let text = |k: &str| values.get(k).and_then(scalar).filter(|s| !s.is_empty());
        let folder = path.rsplit_once('/').map_or("", |(d, _)| d).to_owned();
        let title = text("title").unwrap_or_else(|| {
            folder
                .rsplit('/')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("Untitled project")
                .to_owned()
        });
        Self {
            path: path.to_owned(),
            folder,
            title,
            project_type: text(TYPE_KEY),
            state: text("state")
                .or_else(|| text("status"))
                .unwrap_or_else(|| State::Planning.as_str().to_owned()),
            start: text("start"),
            due: text("due"),
            up: text("up").or_else(|| text("area")),
            members: values.get("members").map(list_of).unwrap_or_default(),
            icon: text("icon"),
            tags: values.get("tags").map(list_of).unwrap_or_default(),
        }
    }

    #[must_use]
    pub fn state(&self) -> State {
        State::parse(&self.state)
    }

    /// A link to it that the vault and Obsidian resolve: by path, shown
    /// as its title.
    #[must_use]
    pub fn link(&self) -> String {
        link_to(&self.path, Some(&self.title))
    }
}

/// Whether the note at `path` is a project's page.
#[must_use]
pub fn is_project_page(path: &str) -> bool {
    path == PROJECT_PAGE || path.ends_with(&format!("/{PROJECT_PAGE}"))
}

/// The project `link` names, among `projects`: a link or path to its
/// `project.md` or to its folder, its folder's name, or its title.
#[must_use]
pub fn resolve<'a>(link: &str, projects: &'a [Project]) -> Option<&'a Project> {
    let target = link_target(link);
    let target = target
        .strip_suffix(".md")
        .unwrap_or(target)
        .trim_end_matches('/');
    let page = |p: &Project| p.path.strip_suffix(".md").unwrap_or(&p.path).to_owned();
    projects
        .iter()
        .find(|p| page(p).eq_ignore_ascii_case(target) || p.folder.eq_ignore_ascii_case(target))
        .or_else(|| {
            projects.iter().find(|p| {
                p.folder
                    .rsplit('/')
                    .next()
                    .is_some_and(|n| n.eq_ignore_ascii_case(target))
                    || p.title.eq_ignore_ascii_case(target)
            })
        })
}

/// Goal kinds, Task's.
pub const GOAL_KINDS: &[&str] = &[
    "lifetime",
    "yearly",
    "quarterly",
    "cycle",
    "weekly",
    "other",
];

/// Goal statuses, Task's, the open ones first.
pub const GOAL_STATUSES: &[&str] = &["aspiration", "active", "paused", "achieved", "abandoned"];

/// One goal.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Goal {
    pub path: String,
    pub title: String,
    pub kind: String,
    pub status: String,
    pub parent: Option<String>,
    pub target_date: Option<String>,
    pub project: Option<String>,
    pub tags: Vec<String>,
}

impl Goal {
    #[must_use]
    pub fn from_values(path: &str, name: &str, values: &Map<String, Value>) -> Self {
        let text = |k: &str| values.get(k).and_then(scalar).filter(|s| !s.is_empty());
        Self {
            path: path.to_owned(),
            title: text("title").unwrap_or_else(|| name.to_owned()),
            kind: goal_kind(text("kind").as_deref().unwrap_or("other")).to_owned(),
            status: goal_status(text("status").as_deref().unwrap_or("aspiration")).to_owned(),
            parent: text("parentId").or_else(|| text("parent")),
            target_date: text("targetDate").or_else(|| text("target")),
            project: text("project"),
            tags: values.get("tags").map(list_of).unwrap_or_default(),
        }
    }

    /// Whether it is still being worked towards.
    #[must_use]
    pub fn is_open(&self) -> bool {
        !matches!(self.status.as_str(), "achieved" | "abandoned")
    }
}

/// A goal kind's canonical name (`year` → `yearly`); unknown is `other`.
#[must_use]
pub fn goal_kind(s: &str) -> &'static str {
    match s.trim().to_ascii_lowercase().as_str() {
        "lifetime" | "life" => "lifetime",
        "yearly" | "year" | "annual" => "yearly",
        "quarterly" | "quarter" => "quarterly",
        "cycle" | "cyclic" => "cycle",
        "weekly" | "week" => "weekly",
        _ => "other",
    }
}

/// A goal status's canonical name (`done` → `achieved`); unknown is an
/// aspiration.
#[must_use]
pub fn goal_status(s: &str) -> &'static str {
    match s.trim().to_ascii_lowercase().as_str() {
        "active" | "in_progress" | "in-progress" => "active",
        "paused" | "on_hold" | "on-hold" => "paused",
        "achieved" | "done" | "complete" | "completed" => "achieved",
        "abandoned" | "dropped" | "cancelled" | "canceled" => "abandoned",
        _ => "aspiration",
    }
}

/// One milestone.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Milestone {
    pub path: String,
    pub title: String,
    pub project: Option<String>,
    pub goal: Option<String>,
    /// `open` or `closed`.
    pub status: String,
    pub due_date: Option<String>,
    pub tags: Vec<String>,
}

impl Milestone {
    #[must_use]
    pub fn from_values(path: &str, name: &str, values: &Map<String, Value>) -> Self {
        let text = |k: &str| values.get(k).and_then(scalar).filter(|s| !s.is_empty());
        Self {
            path: path.to_owned(),
            title: text("title").unwrap_or_else(|| name.to_owned()),
            project: text("project").or_else(|| text("projectId")),
            goal: text("goal").or_else(|| text("goalId")),
            status: milestone_status(text("status").as_deref().unwrap_or("open")).to_owned(),
            due_date: text("dueDate").or_else(|| text("due")),
            tags: values.get("tags").map(list_of).unwrap_or_default(),
        }
    }

    #[must_use]
    pub fn is_open(&self) -> bool {
        self.status == "open"
    }
}

/// `open` or `closed`, Task's reading of the synonyms.
#[must_use]
pub fn milestone_status(s: &str) -> &'static str {
    match s.trim().to_ascii_lowercase().as_str() {
        "closed" | "done" | "shipped" | "complete" | "completed" => "closed",
        _ => "open",
    }
}

/// Whether a note's `project` link (or its place) puts it in `project`.
#[must_use]
pub fn belongs(note_path: &str, link: Option<&str>, project: &Project) -> bool {
    (!project.folder.is_empty() && note_path.starts_with(&format!("{}/", project.folder)))
        || link.is_some_and(|l| {
            let t = link_target(l);
            let t = t.strip_suffix(".md").unwrap_or(t);
            let page = project.path.strip_suffix(".md").unwrap_or(&project.path);
            t.eq_ignore_ascii_case(page) || t.eq_ignore_ascii_case(&project.folder)
        })
}

/// A frontmatter value as text.
#[must_use]
pub fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// A frontmatter value as a list: a sequence's items, or one string.
#[must_use]
pub fn list_of(v: &Value) -> Vec<String> {
    match v {
        Value::Array(items) => items.iter().filter_map(scalar).collect(),
        Value::String(s) if !s.is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

/// What a link points at: `[[a/b|B]]` → `a/b`.
#[must_use]
pub fn link_target(link: &str) -> &str {
    let inner = link
        .trim()
        .strip_prefix("[[")
        .and_then(|l| l.strip_suffix("]]"))
        .unwrap_or_else(|| link.trim());
    let inner = inner.split_once('|').map_or(inner, |(t, _)| t);
    inner.split_once('#').map_or(inner, |(t, _)| t).trim()
}

/// What a link shows: its alias, else its target's last part.
#[must_use]
pub fn link_label(link: &str) -> String {
    let inner = link
        .trim()
        .strip_prefix("[[")
        .and_then(|l| l.strip_suffix("]]"))
        .unwrap_or_else(|| link.trim());
    match inner.split_once('|') {
        Some((_, alias)) => alias.trim().to_owned(),
        None => inner.rsplit('/').next().unwrap_or(inner).to_owned(),
    }
}

/// A link to the note at `path`, shown as `alias` when given.
#[must_use]
pub fn link_to(path: &str, alias: Option<&str>) -> String {
    let target = path.strip_suffix(".md").unwrap_or(path);
    match alias {
        Some(a) if !a.is_empty() => format!("[[{target}|{a}]]"),
        _ => format!("[[{target}]]"),
    }
}

/// A folder or file name for a title, kept readable.
#[must_use]
pub fn file_name(title: &str) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| {
            if "/\\:*?\"<>|#^[]".contains(c) {
                '-'
            } else {
                c
            }
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').trim();
    if cleaned.is_empty() {
        "Untitled".to_owned()
    } else {
        cleaned.chars().take(120).collect()
    }
}

/// A template's text for a new project: `{{title}}` is its title.
#[must_use]
pub fn fill(template: &str, title: &str) -> String {
    template.replace("{{title}}", title)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn values(v: Value) -> Map<String, Value> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn a_project_reads_the_kinds_state_and_the_older_status() {
        let p = Project::from_values(
            "Projects/Site/project.md",
            &values(json!({ "status": "active", "projectType": "software", "members": "[[Sam]]" })),
        );
        assert_eq!(p.title, "Site", "its folder's name without a title");
        assert_eq!(p.folder, "Projects/Site");
        assert_eq!(p.state(), State::Active);
        assert_eq!(p.members, ["[[Sam]]"]);
        assert_eq!(p.link(), "[[Projects/Site/project|Site]]");
        assert_eq!(State::parse("paused"), State::OnHold);
        assert_eq!(State::parse("whatever"), State::Planning);
    }

    #[test]
    fn links_resolve_to_projects_many_ways() {
        let ps = [
            Project::from_values(
                "Projects/Site/project.md",
                &values(json!({ "title": "Website" })),
            ),
            Project::from_values("Projects/Album/Mixing/project.md", &Map::new()),
        ];
        for link in [
            "[[Projects/Site/project|Website]]",
            "Projects/Site/project.md",
            "Projects/Site",
            "[[Site]]",
            "Website",
        ] {
            assert_eq!(
                resolve(link, &ps).map(|p| p.path.as_str()),
                Some("Projects/Site/project.md"),
                "{link}"
            );
        }
        assert_eq!(resolve("Mixing", &ps).unwrap().title, "Mixing");
        assert!(resolve("[[Nope]]", &ps).is_none());
    }

    #[test]
    fn goals_and_milestones_read_tasks_names_and_synonyms() {
        let g = Goal::from_values(
            "Goals/House.md",
            "House",
            &values(json!({ "kind": "year", "status": "done", "targetDate": "2027-01-01" })),
        );
        assert_eq!((g.kind.as_str(), g.status.as_str()), ("yearly", "achieved"));
        assert!(!g.is_open());
        let m = Milestone::from_values(
            "Projects/Site/Milestones/Beta.md",
            "Beta",
            &values(json!({ "status": "shipped", "dueDate": "2026-12-01" })),
        );
        assert_eq!(m.status, "closed");
        assert_eq!(m.due_date.as_deref(), Some("2026-12-01"));
        let site = Project::from_values("Projects/Site/project.md", &Map::new());
        assert!(belongs(&m.path, None, &site), "inside its folder");
        assert!(belongs(
            "Elsewhere/x.md",
            Some("[[Projects/Site/project|Site]]"),
            &site
        ));
        assert!(!belongs("Projects/Sitemap/x.md", None, &site));
    }

    #[test]
    fn labels_and_names() {
        assert_eq!(link_label("[[People/Sam Okafor]]"), "Sam Okafor");
        assert_eq!(link_label("[[a/b|Bee]]"), "Bee");
        assert_eq!(file_name("Q1: plan / ship"), "Q1- plan - ship");
        assert_eq!(fill("# {{title}}\n", "Site"), "# Site\n");
        assert!(
            is_project_page("Projects/Site/project.md")
                && !is_project_page("Projects/Site/notes.md")
        );
    }
}
