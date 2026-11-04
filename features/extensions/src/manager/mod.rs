//! The extensions manager (feature `manager`): one place where the person
//! — or an agent — manages everything that customizes an architect app,
//! so no app builds its own.
//!
//! Everything it manages is an [`Item`] of a kind; each kind is an
//! [`ItemKind`] registered on the [`Manager`]: it lists its items, says
//! what each one is ([`Details`]: what it adds, capabilities and grants,
//! versions, where it runs, its log, its problems, its source) and runs
//! the operations it offers ([`Op`]: enable, disable, remove, update,
//! clone to edit, revoke, …). The kinds that come with it
//! ([`kinds`]):
//!
//! | kind | items | ops |
//! |---|---|---|
//! | `extension` | every extension — built in or linked, installed, in development, offered by a registry — with its runner and which copy runs | install (with consent), enable, disable, update, pin, clone, remove, revoke, back to the built-in |
//! | `script` | each Luau script of a loaded extension: its actions' and hooks' | clone (its extension, to edit) |
//! | `worker` | each process extension's process | restart, stop |
//! | `theme` | every theme in the app's `ThemeStore`: built in, a file, an extension's; its problems | use, clone (a file's), remove (a file's) |
//! | `layout` | `layouts/<name>.json` and `keys.json` (`Home`), built-ins read-only, a broken file's `.error` | use, clone, remove |
//!
//! `architect-pages` adds `pages` (`architect_pages::manage::PagesKind`):
//! every view's page file and override, its status and problems; check,
//! clone to edit, remove the override.
//!
//! **An app adds a kind** by implementing [`ItemKind`] and calling
//! [`Manager::register`] (Studio's audio plugins, a `QuickJS` page kind,
//! a WASI component kind): its items show in the same list and search, its
//! details on the same page, its ops as the same buttons and as
//! `MANAGER_RUN` for agents.
//!
//! **For agents** every operation is an action (`MANAGER_KINDS`,
//! `MANAGER_LIST {kind?, query?}`, `MANAGER_GET {kind, id}`,
//! `MANAGER_RUN {kind, id, op, args?}`); a destructive op from a caller
//! from outside needs a person's yes ([`Manager::confirm_on`]). The
//! `EXT_*` and `THEME_*` actions stay what they are; these list and act
//! across kinds.
//!
//! **The manager is itself an extension** ([`extension::ManagerExtension`],
//! a `linked::Extension`): an app turns it on with
//! `Extensions::with_linked`, and a development copy of `manager` can
//! override it. Its page is `crate::page` (feature `page`).

pub mod extension;
pub mod kinds;

use std::sync::{Arc, PoisonError, RwLock};

use serde::Serialize;
use serde_json::{Value, json};

/// One operation an item offers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Op {
    /// What `MANAGER_RUN` takes as `op` (`enable`, `remove`, `clone`).
    pub id: String,
    /// The button's words.
    pub label: String,
    /// It cannot be taken back (a person confirms it).
    pub destructive: bool,
}

impl Op {
    #[must_use]
    pub fn new(id: &str, label: &str) -> Self {
        Self {
            id: id.to_owned(),
            label: label.to_owned(),
            destructive: false,
        }
    }

    #[must_use]
    pub const fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

/// One thing the manager manages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Item {
    /// Its kind's id (`extension`, `theme`).
    pub kind: String,
    /// Its id within the kind.
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    /// Where it comes from, in words (`built in`, `a file`, `lab.clicker`).
    pub source: String,
    /// On or off, for what can be either.
    pub enabled: Option<bool>,
    /// One line about it.
    pub summary: String,
    /// What is wrong with it now.
    pub problems: Vec<String>,
    /// What can be done to it.
    pub ops: Vec<Op>,
}

impl Item {
    #[must_use]
    pub fn new(kind: &str, id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: kind.to_owned(),
            id: id.into(),
            name: name.into(),
            version: None,
            source: String::new(),
            enabled: None,
            summary: String::new(),
            problems: Vec::new(),
            ops: Vec::new(),
        }
    }

    /// Whether it has every word of `query` (in its name, id, kind,
    /// source or summary).
    #[must_use]
    pub fn matches(&self, query: &str) -> bool {
        let hay = format!(
            "{} {} {} {} {}",
            self.name, self.id, self.kind, self.source, self.summary
        )
        .to_lowercase();
        query
            .split_whitespace()
            .all(|w| hay.contains(&w.to_lowercase()))
    }
}

/// A part of an item's page: a title and its lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Part {
    pub title: String,
    pub lines: Vec<String>,
}

impl Part {
    #[must_use]
    pub fn new(title: &str, lines: Vec<String>) -> Self {
        Self {
            title: title.to_owned(),
            lines,
        }
    }
}

/// Everything about one item: its page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Details {
    pub item: Item,
    /// What it adds, what it asks for, where it runs, its versions, its
    /// log — each kind says what it has.
    pub parts: Vec<Part>,
    /// The kind's own data, for an agent (`EXT_INFO`'s answer for an
    /// extension).
    pub data: Value,
}

/// One kind of thing the manager manages.
pub trait ItemKind: Send + Sync {
    /// Its id (`extension`), what `MANAGER_*` take as `kind`.
    fn id(&self) -> &str;

    /// Its name on the page (`Extensions`).
    fn title(&self) -> &str;

    /// Every item of it now.
    fn list(&self) -> Vec<Item>;

    /// Item `id`'s page: its item and no parts unless the kind says more.
    fn get(&self, id: &str) -> Option<Details> {
        self.list()
            .into_iter()
            .find(|i| i.id == id)
            .map(|item| Details {
                item,
                parts: Vec::new(),
                data: Value::Null,
            })
    }

    /// Runs op `op` on item `id` with `args`: what it did.
    ///
    /// # Errors
    ///
    /// No such item or op, or the op failed.
    fn run(&self, id: &str, op: &str, args: &Value) -> Result<Value, String>;
}

/// Every kind an app manages. Cheap to clone: every clone is the same.
#[derive(Clone, Default)]
pub struct Manager {
    kinds: Arc<RwLock<Vec<Arc<dyn ItemKind>>>>,
    /// Where [`Manager::show`] goes: the page's, while it is up.
    page: Arc<std::sync::Mutex<Option<futures::channel::mpsc::UnboundedSender<(String, String)>>>>,
}

impl PartialEq for Manager {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.kinds, &other.kinds)
    }
}

impl std::fmt::Debug for Manager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.kinds().iter().map(|k| k.id().to_owned()))
            .finish()
    }
}

impl Manager {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds `kind` (in place of one by its id).
    pub fn register(&self, kind: impl ItemKind + 'static) {
        let kind: Arc<dyn ItemKind> = Arc::new(kind);
        let mut kinds = self.kinds.write().unwrap_or_else(PoisonError::into_inner);
        kinds.retain(|k| k.id() != kind.id());
        kinds.push(kind);
    }

    /// [`Self::register`], chained.
    #[must_use]
    pub fn with(self, kind: impl ItemKind + 'static) -> Self {
        self.register(kind);
        self
    }

    /// Opens the page at item `id` of `kind` (`MANAGER_SHOW`, ⌘O's `ext:`):
    /// false when no page is up to show it.
    #[must_use = "false when there is no page"]
    pub fn show(&self, kind: &str, id: &str) -> bool {
        self.page
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .is_some_and(|tx| tx.unbounded_send((kind.to_owned(), id.to_owned())).is_ok())
    }

    /// What [`Self::show`] asks for, from now on: the page's.
    #[must_use = "the page reads what it is asked to show from it"]
    pub fn shown(&self) -> futures::channel::mpsc::UnboundedReceiver<(String, String)> {
        let (tx, rx) = futures::channel::mpsc::unbounded();
        *self.page.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
        rx
    }

    /// The kinds, in the order registered.
    #[must_use]
    pub fn kinds(&self) -> Vec<Arc<dyn ItemKind>> {
        self.kinds
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Kind `id`.
    #[must_use]
    pub fn kind(&self, id: &str) -> Option<Arc<dyn ItemKind>> {
        self.kinds().into_iter().find(|k| k.id() == id)
    }

    /// Every item of `kind` (every kind's when `None`) matching `query`.
    #[must_use]
    pub fn list(&self, kind: Option<&str>, query: &str) -> Vec<Item> {
        self.kinds()
            .into_iter()
            .filter(|k| kind.is_none_or(|w| w == k.id()))
            .flat_map(|k| k.list())
            .filter(|i| i.matches(query))
            .collect()
    }

    /// Item `id` of `kind`'s page.
    #[must_use]
    pub fn get(&self, kind: &str, id: &str) -> Option<Details> {
        self.kind(kind)?.get(id)
    }

    /// Runs `op` on item `id` of `kind`.
    ///
    /// # Errors
    ///
    /// No such kind, item or op, or it failed.
    pub fn run(&self, kind: &str, id: &str, op: &str, args: &Value) -> Result<Value, String> {
        let k = self
            .kind(kind)
            .ok_or_else(|| format!("no kind {kind} (MANAGER_KINDS lists them)"))?;
        let span = tracing::info_span!(
            "manager.run",
            manager.kind = kind,
            manager.id = id,
            manager.op = op
        );
        let _entered = span.enter();
        k.run(id, op, args)
    }

    /// Whether `op` on item `id` of `kind` is destructive.
    #[must_use]
    pub fn is_destructive(&self, kind: &str, id: &str, op: &str) -> bool {
        self.kind(kind)
            .and_then(|k| k.list().into_iter().find(|i| i.id == id))
            .is_some_and(|i| i.ops.iter().any(|o| o.id == op && o.destructive))
    }

    /// The kinds and how many items each has, with their problems' count.
    #[must_use]
    pub fn summary(&self) -> Value {
        let kinds: Vec<Value> = self
            .kinds()
            .iter()
            .map(|k| {
                let items = k.list();
                json!({
                    "kind": k.id(),
                    "title": k.title(),
                    "items": items.len(),
                    "problems": items.iter().map(|i| i.problems.len()).sum::<usize>(),
                })
            })
            .collect();
        json!({ "kinds": kinds })
    }

    /// A destructive op from a caller from outside needs a person's yes
    /// on `list` (where `MANAGER_RUN` is offered).
    pub fn confirm_on(&self, list: &architect::action::ActionList) {
        let me = self.clone();
        list.set_confirm("MANAGER_RUN", move |args| {
            let Ok(v) = serde_json::from_slice::<Value>(args) else {
                return true;
            };
            let s = |k: &str| {
                v.get(k)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            me.is_destructive(&s("kind"), &s("id"), &s("op"))
        });
    }
}
