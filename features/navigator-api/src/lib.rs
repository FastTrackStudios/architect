//! The navigator's provider API: what fills the sidebar's two panes.
//!
//! The navigator (`architect-navigator`, a port of Notebook Navigator) is
//! the app's one left sidebar: a navigation pane (a tree of containers)
//! beside a list pane (the items in the selected container). The vault is
//! one provider: folders, and the notes in them. A page or an extension is
//! another: projects and their tasks, setlists and their songs.
//!
//! **Licence.** This crate is MIT OR Apache-2.0. The navigator is
//! GPL-3.0-or-later; a page or an extension implements
//! [`NavigatorProvider`] from here and never links the GPL crate. The app
//! that draws the navigator links it (and ships under the GPL); the
//! provider only hands over data and answers requests.
//!
//! - **Data:** a [`Snapshot`]: its title, the [`Node`] tree and every
//!   [`Item`], each in one container ([`Item::container`]). The list for a
//!   selection is the items in it (and, with the navigator's "notes from
//!   subfolders" on, in the containers below).
//! - **Requests:** open an item ([`Open`]), move items to another
//!   container (a drag), create one in a container.
//! - **Two ways to serve it.** Linked Rust code implements the trait;
//!   `architect_extensions::linked::Builder::navigator` serves it as
//!   actions. A process extension, a Luau one or a page answers the same
//!   actions itself ([`Method`]: `<PREFIX>_SNAPSHOT`, `_OPEN`, `_MOVE`,
//!   `_CREATE`), declared in its manifest's `[[navigator_providers]]`. The
//!   app reads every provider through those actions ([`ActionProvider`]),
//!   so it treats them all alike.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A project's glyph, a Lucide name.
///
/// It shows wherever a project does: the navigator's Projects row and its
/// projects, sub-projects, a project's folder in a vault, its
/// `project.md`, its deck's tab. It replaces the folder icon; nothing
/// folder-shaped is in it. A project's own `icon`
/// (and `color`), when its `project.md` sets one, replaces it in turn.
pub const PROJECT_ICON: &str = "box";

/// Milliseconds since the Unix epoch, UTC.
pub type Millis = i64;

/// What a provider's requests answer with.
pub type ProviderFuture<T> = Pin<Box<dyn Future<Output = Result<T, String>> + Send>>;

/// A date written in a note (`2026-11-05`, `2026-11-05T09:30`,
/// `2026-11-05T09:30:00Z`) as [`Millis`], read as UTC: what an item's
/// `created` and `modified` are made from.
#[must_use]
// Every field is a few digits (a 4-digit year, 2-digit months to
// seconds): nothing here can overflow an i64.
#[allow(clippy::arithmetic_side_effects)]
pub fn date_millis(text: &str) -> Option<Millis> {
    let t = text.trim();
    let num = |a: usize, b: usize| t.get(a..b)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if t.get(4..5) != Some("-")
        || t.get(7..8) != Some("-")
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
    {
        return None;
    }
    let (hh, mm, ss) = if t.get(10..11).is_some_and(|c| c == "T" || c == " ") {
        (
            num(11, 13).unwrap_or(0),
            num(14, 16).unwrap_or(0),
            num(17, 19).unwrap_or(0),
        )
    } else {
        (0, 0, 0)
    };
    // Days from the civil date (Howard Hinnant's `days_from_civil`).
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 24 + hh) * 60 + mm) * 60_000 + ss * 1000)
}

/// One container in the navigation pane: a folder, a project, an area, a
/// setlist.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Node {
    /// Unique among the provider's nodes; what [`Item::container`] names.
    pub id: String,
    pub title: String,
    /// A Lucide icon's name (`folder`, `check`).
    pub icon: Option<String>,
    /// A CSS colour for its row.
    pub colour: Option<String>,
    /// The containers inside it.
    pub children: Vec<Self>,
    /// The org it is from (`codywright`), when the provider lists more
    /// than one org's: the navigator marks its row with the org's badge
    /// (an asset library, a project shared from another org).
    pub org: Option<String>,
    /// Shared into the person's org from another (a project in a Space):
    /// its row says so.
    pub shared: bool,
}

impl Node {
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    #[must_use]
    pub fn colour(mut self, colour: impl Into<String>) -> Self {
        self.colour = Some(colour.into());
        self
    }

    /// It is org `org`'s ([`Self::org`]).
    #[must_use]
    pub fn org(mut self, org: impl Into<String>) -> Self {
        self.org = Some(org.into());
        self
    }

    /// It is shared from another org ([`Self::shared`]).
    #[must_use]
    pub const fn shared(mut self) -> Self {
        self.shared = true;
        self
    }

    #[must_use]
    pub fn child(mut self, node: Self) -> Self {
        self.children.push(node);
        self
    }

    /// It and every node below it, depth first, each with its parent's id
    /// (`None` at the top).
    pub fn walk<'a>(&'a self, parent: Option<&'a str>, out: &mut Vec<(Option<&'a str>, &'a Self)>) {
        out.push((parent, self));
        for c in &self.children {
            c.walk(Some(&self.id), out);
        }
    }
}

/// An item's tasks: how many, and how many are done.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

/// One row of the list pane: a note, a task, a song.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Item {
    /// Unique among the provider's items.
    pub id: String,
    /// The [`Node::id`] it is in; `""` is the root (the provider itself).
    pub container: String,
    pub title: String,
    /// A line or two under the title.
    pub preview: Option<String>,
    /// When it was made and last changed ([`Millis`]): the list sorts and
    /// groups by them.
    pub created: Millis,
    pub modified: Millis,
    /// A Lucide icon's name.
    pub icon: Option<String>,
    /// An image beside it (a vault path or a URL).
    pub thumbnail: Option<String>,
    /// Its tags, without `#`.
    pub tags: Vec<String>,
    /// Its tasks' progress (a task: 0 or 1 of 1).
    pub progress: Option<Progress>,
    /// Short words shown with it (`overdue`, `high`).
    pub badges: Vec<String>,
    /// Name and value pairs the list can show, and group and sort by.
    pub properties: Vec<(String, String)>,
    /// What opening it does, when the provider knows without being asked;
    /// `None` asks it ([`NavigatorProvider::open`]).
    pub open: Option<Open>,
}

impl Item {
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        container: impl Into<String>,
        title: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            container: container.into(),
            title: title.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn preview(mut self, text: impl Into<String>) -> Self {
        self.preview = Some(text.into());
        self
    }

    #[must_use]
    pub const fn dates(mut self, created: Millis, modified: Millis) -> Self {
        self.created = created;
        self.modified = modified;
        self
    }

    /// A Lucide icon's name for its row (`calendar`, `timer`).
    #[must_use]
    pub fn icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    #[must_use]
    pub fn tag(mut self, tag: impl Into<String>) -> Self {
        self.tags.push(tag.into());
        self
    }

    #[must_use]
    pub fn badge(mut self, badge: impl Into<String>) -> Self {
        self.badges.push(badge.into());
        self
    }

    #[must_use]
    pub fn property(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.properties.push((key.into(), value.into()));
        self
    }

    #[must_use]
    pub const fn progress(mut self, done: u32, total: u32) -> Self {
        self.progress = Some(Progress { done, total });
        self
    }

    #[must_use]
    pub fn opens(mut self, open: Open) -> Self {
        self.open = Some(open);
        self
    }
}

/// What the app does to open an item, in its workspace.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Open {
    /// Nothing (the provider did it itself).
    #[default]
    Nothing,
    /// A file of the vault, at its path.
    File { path: String },
    /// One of an extension's views, given `state`.
    View {
        extension: String,
        view: String,
        #[serde(default)]
        state: Value,
    },
    /// An action, with `args`.
    Action {
        id: String,
        #[serde(default)]
        args: Value,
    },
    /// A context loaded onto a deck (a song, a project): a click loads it
    /// onto the deck on show, a ⌘-click or a middle-click onto a new one
    /// (`architect_workspace::DeckContext`; `DECK_OPEN`).
    Context {
        /// Its type (`song`, `project`): its default screenset.
        context_type: String,
        id: String,
        title: String,
        /// Its own folder, when it has one.
        #[serde(default)]
        folder: Option<String>,
    },
}

/// Everything a provider shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Snapshot {
    /// The root's row (`Projects`).
    pub title: String,
    pub tree: Vec<Node>,
    pub items: Vec<Item>,
    /// The nodes show in the order given (Today, Tomorrow, Later), not by
    /// name, until the person orders them by hand.
    pub ordered: bool,
    /// A number on its row in the navigator's left list: what is still to
    /// do (the Queue's captures to process).
    pub count: Option<u32>,
    /// Its nodes are the list's groups, not rows of the tree: its row
    /// selected lists every item, under its node's heading, in the nodes'
    /// order (the Queue's Inbox, Today, Upcoming, Waiting).
    pub groups: bool,
    /// Its lists grouped by this item property (`section`), its groups in
    /// [`Self::group_order`] first.
    pub group_by: Option<String>,
    pub group_order: Vec<String>,
    /// A month grid under its row, over its tree (the Calendar's, #444):
    /// a day clicked selects that day's node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calendar: Option<CalendarGrid>,
}

/// One day of a provider's month grid ([`Snapshot::calendar`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GridDay {
    /// The node a click on it selects (`d:2026-11-07`).
    pub node: String,
    /// How many things are on it: its dots (at most three drawn).
    pub count: u32,
    /// A word for it, its tooltip and its mark (the day's template:
    /// `Workday`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// A CSS colour for the mark.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
}

/// A provider's month grid: its days, by ISO date (`2026-11-07`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CalendarGrid {
    pub days: std::collections::BTreeMap<String, GridDay>,
}

impl Snapshot {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn node(mut self, node: Node) -> Self {
        self.tree.push(node);
        self
    }

    #[must_use]
    pub fn item(mut self, item: Item) -> Self {
        self.items.push(item);
        self
    }

    /// Its nodes keep the order given ([`Self::ordered`]).
    #[must_use]
    pub const fn ordered(mut self) -> Self {
        self.ordered = true;
        self
    }

    /// The number on its row ([`Self::count`]).
    #[must_use]
    pub const fn count(mut self, n: u32) -> Self {
        self.count = Some(n);
        self
    }

    /// Its nodes are the list's groups, in order ([`Self::groups`]).
    #[must_use]
    pub const fn grouped(mut self) -> Self {
        self.groups = true;
        self.ordered = true;
        self
    }

    /// Its lists grouped by item property `key`, `order` first.
    #[must_use]
    pub fn group_by<S: Into<String>>(
        mut self,
        key: impl Into<String>,
        order: impl IntoIterator<Item = S>,
    ) -> Self {
        self.group_by = Some(key.into());
        self.group_order = order.into_iter().map(Into::into).collect();
        self
    }

    /// A month grid under its row ([`Self::calendar`]).
    #[must_use]
    pub fn calendar(mut self, grid: CalendarGrid) -> Self {
        self.calendar = Some(grid);
        self
    }

    /// Every node, depth first, with its parent's id.
    #[must_use]
    pub fn nodes(&self) -> Vec<(Option<&str>, &Node)> {
        let mut out = Vec::new();
        for n in &self.tree {
            n.walk(None, &mut out);
        }
        out
    }
}

/// What fills the navigator's two panes: a tree of containers, and the
/// items in them.
///
/// Implemented by linked Rust code; served as actions by
/// `Builder::navigator` ([`serve`]). Every request may take its time (it
/// may read the vault through the host); the navigator draws the last
/// snapshot meanwhile.
pub trait NavigatorProvider: Send + Sync + 'static {
    /// The tree and every item, now.
    fn snapshot(&self) -> ProviderFuture<Snapshot>;

    /// Opens item `item` (one whose [`Item::open`] is `None`): what the
    /// app does next. Nothing, by default.
    fn open(&self, item: String) -> ProviderFuture<Open> {
        let _ = item;
        Box::pin(async { Ok(Open::Nothing) })
    }

    /// Moves `items` into container `to` (a drag onto a node).
    ///
    /// # Errors
    ///
    /// By default: the provider's items do not move.
    fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
        let _ = (items, to);
        Box::pin(async { Err("these items do not move".to_owned()) })
    }

    /// Makes an item called `title` in container `container` (the list's
    /// "new" button): what to open then.
    ///
    /// # Errors
    ///
    /// By default: nothing is made here.
    fn create(&self, container: String, title: String) -> ProviderFuture<Open> {
        let _ = (container, title);
        Box::pin(async { Err("nothing can be made here".to_owned()) })
    }
}

/// One of a provider's requests, as an action: `<PREFIX>_<METHOD>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `{}` → a [`Snapshot`].
    Snapshot,
    /// `{"item"}` → an [`Open`].
    Open,
    /// `{"items": [..], "to"}` → `null`.
    Move,
    /// `{"container", "title"}` → an [`Open`].
    Create,
}

impl Method {
    pub const ALL: [Self; 4] = [Self::Snapshot, Self::Open, Self::Move, Self::Create];

    /// Its action's suffix.
    #[must_use]
    pub const fn suffix(self) -> &'static str {
        match self {
            Self::Snapshot => "SNAPSHOT",
            Self::Open => "OPEN",
            Self::Move => "MOVE",
            Self::Create => "CREATE",
        }
    }

    /// Its action, for a provider whose actions start with `prefix`
    /// (`PROJECTS_NAV` → `PROJECTS_NAV_SNAPSHOT`).
    #[must_use]
    pub fn action(self, prefix: &str) -> String {
        format!("{prefix}_{}", self.suffix())
    }
}

fn text(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("give `{key}`"))
}

/// Answers `method` of `provider`, given its action's `args`: what
/// `Builder::navigator` registers for each of its actions.
pub fn serve(
    provider: Arc<dyn NavigatorProvider>,
    method: Method,
    args: Value,
) -> ProviderFuture<Value> {
    Box::pin(async move {
        match method {
            Method::Snapshot => to_value(provider.snapshot().await?),
            Method::Open => to_value(provider.open(text(&args, "item")?).await?),
            Method::Move => {
                let items = args
                    .get("items")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .ok_or("give `items`")?;
                provider.move_items(items, text(&args, "to")?).await?;
                Ok(Value::Null)
            }
            Method::Create => to_value(
                provider
                    .create(text(&args, "container")?, text(&args, "title")?)
                    .await?,
            ),
        }
    })
}

fn to_value(v: impl Serialize) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn from_value<T: for<'de> Deserialize<'de>>(v: Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

/// Runs an action: its id and arguments, its answer.
pub type Run = Arc<dyn Fn(String, Value) -> ProviderFuture<Value> + Send + Sync>;

/// A provider read through its actions: how the app sees every provider,
/// linked or a process, so it treats them all alike.
#[derive(Clone)]
pub struct ActionProvider {
    prefix: String,
    run: Run,
}

impl ActionProvider {
    /// The provider whose actions start with `prefix`, run by `run`.
    #[must_use]
    pub fn new(prefix: impl Into<String>, run: Run) -> Self {
        Self {
            prefix: prefix.into(),
            run,
        }
    }

    fn call(&self, method: Method, args: Value) -> ProviderFuture<Value> {
        (self.run)(method.action(&self.prefix), args)
    }
}

impl std::fmt::Debug for ActionProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionProvider")
            .field("prefix", &self.prefix)
            .finish_non_exhaustive()
    }
}

impl NavigatorProvider for ActionProvider {
    fn snapshot(&self) -> ProviderFuture<Snapshot> {
        let call = self.call(Method::Snapshot, Value::Null);
        Box::pin(async move { from_value(call.await?) })
    }

    fn open(&self, item: String) -> ProviderFuture<Open> {
        let call = self.call(Method::Open, json!({ "item": item }));
        Box::pin(async move { from_value(call.await?) })
    }

    fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
        let call = self.call(Method::Move, json!({ "items": items, "to": to }));
        Box::pin(async move { call.await.map(|_| ()) })
    }

    fn create(&self, container: String, title: String) -> ProviderFuture<Open> {
        let call = self.call(
            Method::Create,
            json!({ "container": container, "title": title }),
        );
        Box::pin(async move { from_value(call.await?) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Two projects, a task in each; a move is kept.
    struct Board(Mutex<Vec<Item>>);

    impl NavigatorProvider for Board {
        fn snapshot(&self) -> ProviderFuture<Snapshot> {
            let items = self.0.lock().map(|i| i.clone()).unwrap_or_default();
            Box::pin(async move {
                Ok(Snapshot {
                    title: "Projects".to_owned(),
                    tree: vec![
                        Node::new("a", "Alpha").icon("folder"),
                        Node::new("b", "Beta").child(Node::new("b1", "Beta one")),
                    ],
                    items,
                    ordered: false,
                    ..Snapshot::default()
                })
            })
        }

        fn move_items(&self, items: Vec<String>, to: String) -> ProviderFuture<()> {
            if let Ok(mut all) = self.0.lock() {
                for i in all.iter_mut().filter(|i| items.contains(&i.id)) {
                    i.container.clone_from(&to);
                }
            }
            Box::pin(async { Ok(()) })
        }
    }

    fn board() -> Arc<dyn NavigatorProvider> {
        Arc::new(Board(Mutex::new(vec![
            Item::new("t1", "a", "Write it").progress(0, 1),
            Item::new("t2", "b", "Ship it").opens(Open::File {
                path: "Tasks/Ship it.md".to_owned(),
            }),
        ])))
    }

    /// The trait served as actions and read back through them is the
    /// same provider: what a process extension and the app do.
    #[test]
    fn actions_round_trip() {
        let linked = board();
        let served = Arc::clone(&linked);
        let run: Run = Arc::new(move |action: String, args: Value| {
            let method = Method::ALL
                .into_iter()
                .find(|m| m.action("PROJECTS_NAV") == action);
            let served = Arc::clone(&served);
            Box::pin(async move { serve(served, method.ok_or("no such action")?, args).await })
        });
        let remote = ActionProvider::new("PROJECTS_NAV", run);
        let direct = futures::executor::block_on(linked.snapshot()).ok();
        let read = futures::executor::block_on(remote.snapshot()).ok();
        assert_eq!(direct, read);
        assert_eq!(read.as_ref().map(|s| s.nodes().len()), Some(3));
        assert!(
            futures::executor::block_on(remote.move_items(vec!["t1".to_owned()], "b1".to_owned()))
                .is_ok()
        );
        let after = futures::executor::block_on(remote.snapshot()).unwrap_or_default();
        assert_eq!(
            after.items.first().map(|i| i.container.as_str()),
            Some("b1")
        );
        // Not one it answers: the default's error, through the action.
        assert!(
            futures::executor::block_on(remote.create("a".to_owned(), "New".to_owned())).is_err()
        );
        assert_eq!(
            futures::executor::block_on(remote.open("t1".to_owned())),
            Ok(Open::Nothing)
        );
    }

    #[test]
    fn dates_read_as_utc_millis() {
        assert_eq!(date_millis("1970-01-01"), Some(0));
        assert_eq!(date_millis("2026-11-05"), Some(1_793_836_800_000));
        assert_eq!(
            date_millis("2026-11-05T12:00:00Z"),
            Some(1_793_836_800_000 + 43_200_000)
        );
        assert_eq!(date_millis("soon"), None);
    }

    #[test]
    fn open_reads_as_tagged_json() {
        let v = serde_json::to_value(Open::File {
            path: "a.md".to_owned(),
        })
        .unwrap_or_default();
        assert_eq!(v, json!({ "kind": "file", "path": "a.md" }));
        let o: Open = serde_json::from_value(
            json!({ "kind": "view", "extension": "projects", "view": "page" }),
        )
        .unwrap_or_default();
        assert_eq!(
            o,
            Open::View {
                extension: "projects".to_owned(),
                view: "page".to_owned(),
                state: Value::Null
            }
        );
    }
}
