//! The menu bar (File, Edit, View, …) as a view of the commands
//! (`docs/content/architecture/app-menus.md`): an app says which commands
//! go in which menu, in what order, grouped how; the labels, the keys and
//! whether each can run come from the commands themselves. A backend draws
//! the resolved bar (the native one on macOS: feature `native`).
//!
//! A toggle is ticked when on ([`Command::checked`]). A [`Section`] is
//! live: a function that lists its rows (the recent projects, the
//! screensets, the open windows) each time the bar is resolved, which a
//! backend does when a menu opens or the commands change. [`Tree`] numbers
//! a resolved bar for a backend that names items by number (dbusmenu).

use std::rc::Rc;

use architect_palette::Command;

/// What the platform does itself: the edit commands a text field answers,
/// the window's, the app's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Standard {
    About,
    Services,
    Hide,
    HideOthers,
    ShowAll,
    Quit,
    Undo,
    Redo,
    Cut,
    Copy,
    Paste,
    SelectAll,
    Minimize,
    Zoom,
    FullScreen,
    BringAllToFront,
    CloseWindow,
}

/// One entry of a menu, as the app declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A command by id.
    Command(String),
    /// Every command of a group, in label order.
    Group(String),
    Separator,
    Standard(Standard),
    /// A menu inside the menu.
    Submenu(Menu),
    /// Rows listed when the bar is resolved.
    Section(Section),
}

/// The name of the live section [`MenuBar::contributed`] adds.
pub const CONTRIBUTED: &str = "contributed";

/// One row of a live section: a command, run with its own arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Live {
    pub label: String,
    /// The command it runs (`WORKSPACE_RECALL_SCREENSET`).
    pub command: String,
    /// What it runs it with, as JSON (`{"id":"mixing"}`); `None` for nothing.
    pub args: Option<String>,
    /// Ticked when on (the current screenset); `None` when it is not a toggle.
    pub checked: Option<bool>,
    pub enabled: bool,
    /// Why not, when it cannot run now.
    pub disabled_reason: Option<String>,
}

impl Live {
    pub fn new(label: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            command: command.into(),
            args: None,
            checked: None,
            enabled: true,
            disabled_reason: None,
        }
    }

    /// The command's arguments, as JSON.
    #[must_use]
    pub fn args(mut self, json: impl Into<String>) -> Self {
        self.args = Some(json.into());
        self
    }

    #[must_use]
    pub const fn checked(mut self, on: bool) -> Self {
        self.checked = Some(on);
        self
    }

    #[must_use]
    pub fn disabled(mut self, reason: impl Into<String>) -> Self {
        self.enabled = false;
        self.disabled_reason = Some(reason.into());
        self
    }
}

/// A live section: a function that lists its rows, called each time the
/// bar is resolved. Two sections are the same when they are one function
/// (a clone), so a bar stays equal to itself across renders.
#[derive(Clone)]
pub struct Section {
    pub name: String,
    rows: Rc<dyn Fn() -> Vec<Live>>,
}

impl Section {
    pub fn new(name: impl Into<String>, rows: impl Fn() -> Vec<Live> + 'static) -> Self {
        Self {
            name: name.into(),
            rows: Rc::new(rows),
        }
    }

    /// Its rows now.
    #[must_use]
    pub fn rows(&self) -> Vec<Live> {
        (self.rows)()
    }
}

impl PartialEq for Section {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name && Rc::ptr_eq(&self.rows, &other.rows)
    }
}

impl Eq for Section {}

impl std::fmt::Debug for Section {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Section")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

/// A menu: its title and its entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Menu {
    pub title: String,
    pub entries: Vec<Entry>,
}

impl Menu {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            entries: Vec::new(),
        }
    }

    #[must_use]
    pub fn command(mut self, id: impl Into<String>) -> Self {
        self.entries.push(Entry::Command(id.into()));
        self
    }

    #[must_use]
    pub fn commands<I: IntoIterator<Item = S>, S: Into<String>>(mut self, ids: I) -> Self {
        self.entries
            .extend(ids.into_iter().map(|id| Entry::Command(id.into())));
        self
    }

    /// Every command in `group`, whatever registers them.
    #[must_use]
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.entries.push(Entry::Group(group.into()));
        self
    }

    #[must_use]
    pub fn separator(mut self) -> Self {
        self.entries.push(Entry::Separator);
        self
    }

    #[must_use]
    pub fn standard(mut self, item: Standard) -> Self {
        self.entries.push(Entry::Standard(item));
        self
    }

    /// The command `id` where the platform's `item` was: an app that keeps
    /// its own history puts its Undo in Edit › Undo, so ⌘Z reaches it
    /// outside a text field too.
    #[must_use]
    pub fn instead(mut self, item: Standard, id: impl Into<String>) -> Self {
        let id = id.into();
        for entry in &mut self.entries {
            if *entry == Entry::Standard(item) {
                *entry = Entry::Command(id.clone());
            }
        }
        self
    }

    #[must_use]
    pub fn submenu(mut self, menu: Self) -> Self {
        self.entries.push(Entry::Submenu(menu));
        self
    }

    /// A live section: `rows` lists them each time the bar is resolved.
    #[must_use]
    pub fn section(
        mut self,
        name: impl Into<String>,
        rows: impl Fn() -> Vec<Live> + 'static,
    ) -> Self {
        self.entries.push(Entry::Section(Section::new(name, rows)));
        self
    }
}

/// The whole bar.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MenuBar {
    pub menus: Vec<Menu>,
}

impl MenuBar {
    /// The platform's shape: the app menu (About, Settings, Services,
    /// Hide, Quit), File, Edit (Undo … Select All), View, Window
    /// (Minimize, Zoom, Bring All to Front), Help — each of which the app
    /// fills with [`Self::menu`].
    #[must_use]
    pub fn standard(app: &str) -> Self {
        let app_menu = Menu::new(app)
            .standard(Standard::About)
            .separator()
            .command("APP_SETTINGS")
            .separator()
            .standard(Standard::Services)
            .separator()
            .standard(Standard::Hide)
            .standard(Standard::HideOthers)
            .standard(Standard::ShowAll)
            .separator()
            .standard(Standard::Quit);
        let edit = Menu::new("Edit")
            .standard(Standard::Undo)
            .standard(Standard::Redo)
            .separator()
            .standard(Standard::Cut)
            .standard(Standard::Copy)
            .standard(Standard::Paste)
            .standard(Standard::SelectAll);
        let window = Menu::new("Window")
            .standard(Standard::Minimize)
            .standard(Standard::Zoom)
            .standard(Standard::FullScreen)
            .separator()
            .standard(Standard::BringAllToFront);
        Self {
            menus: vec![
                app_menu,
                Menu::new("File"),
                edit,
                Menu::new("View"),
                window,
                Menu::new("Help"),
            ],
        }
    }

    /// Adds `f`'s entries to the menu titled `title` (made at the end, before
    /// Window and Help, when there is none).
    #[must_use]
    pub fn menu(mut self, title: &str, f: impl FnOnce(Menu) -> Menu) -> Self {
        if let Some(slot) = self.menus.iter_mut().find(|m| m.title == title) {
            let menu = std::mem::replace(slot, Menu::new(""));
            *slot = f(menu);
        } else {
            let at = self
                .menus
                .iter()
                .position(|m| m.title == "Window" || m.title == "Help")
                .unwrap_or(self.menus.len());
            self.menus.insert(at, f(Menu::new(title)));
        }
        self
    }

    /// Every menu with a live section of what others add to it (an
    /// extension's `[[menus]]` rows): `rows` is asked for each menu's by
    /// its title each time the bar is resolved, so rows come and go with
    /// whoever adds them. A menu with none shows as it did.
    #[must_use]
    pub fn contributed(mut self, rows: impl Fn(&str) -> Vec<Live> + 'static) -> Self {
        let rows: Rc<dyn Fn(&str) -> Vec<Live>> = Rc::new(rows);
        for menu in &mut self.menus {
            let (title, rows) = (menu.title.clone(), Rc::clone(&rows));
            menu.entries.push(Entry::Separator);
            menu.entries.push(Entry::Section(Section {
                name: CONTRIBUTED.to_owned(),
                rows: Rc::new(move || rows(&title)),
            }));
        }
        self
    }

    /// Resolved against the commands there are now: each command entry
    /// becomes its label, key and state; a group, its commands; a live
    /// section, its rows now (a row whose command is not there is left
    /// out, one whose command cannot run is disabled with its reason); a
    /// command that is not there is left out, and so is a menu left empty
    /// (the app's own menu always stays). Separators never lead, trail or
    /// double.
    #[must_use]
    pub fn resolve(&self, commands: &[Command]) -> Vec<Resolved> {
        let mut out = Vec::new();
        for (i, menu) in self.menus.iter().enumerate() {
            let items = resolve_entries(&menu.entries, commands);
            if i == 0 || !items.is_empty() {
                out.push(Resolved {
                    title: menu.title.clone(),
                    items,
                });
            }
        }
        out
    }
}

/// A menu as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub title: String,
    pub items: Vec<Item>,
}

/// An item as drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Command(Command),
    Separator,
    Standard(Standard),
    Submenu(Resolved),
    /// A live section's row.
    Live(Live),
}

/// `menus` without the platform's own items, for a bar the app draws.
///
/// Where the app draws or exports the bar (dbusmenu, the in-window bar)
/// there is no Undo, Copy or Quit of the platform's to wire them to. Menus
/// left empty go, the app's own too; separators are tidied again.
#[must_use]
pub fn without_platform(menus: &[Resolved]) -> Vec<Resolved> {
    menus
        .iter()
        .filter_map(|m| {
            let items = own_items(&m.items);
            (!items.is_empty()).then(|| Resolved {
                title: m.title.clone(),
                items,
            })
        })
        .collect()
}

fn own_items(items: &[Item]) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::new();
    for item in items {
        match item {
            Item::Standard(_) => {}
            Item::Separator => {
                if out.last().is_some_and(|i| *i != Item::Separator) {
                    out.push(Item::Separator);
                }
            }
            Item::Submenu(sub) => {
                let inner = own_items(&sub.items);
                if !inner.is_empty() {
                    out.push(Item::Submenu(Resolved {
                        title: sub.title.clone(),
                        items: inner,
                    }));
                }
            }
            other => out.push(other.clone()),
        }
    }
    while out.last() == Some(&Item::Separator) {
        out.pop();
    }
    out
}

fn resolve_entries(entries: &[Entry], commands: &[Command]) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    for entry in entries {
        match entry {
            Entry::Command(id) => {
                if let Some(c) = commands.iter().find(|c| &c.id == id) {
                    items.push(Item::Command(c.clone()));
                }
            }
            Entry::Group(group) => {
                let mut in_group: Vec<&Command> =
                    commands.iter().filter(|c| &c.group == group).collect();
                in_group.sort_by(|a, b| a.label.cmp(&b.label));
                items.extend(in_group.into_iter().cloned().map(Item::Command));
            }
            Entry::Separator => {
                if items.last().is_some_and(|i| *i != Item::Separator) {
                    items.push(Item::Separator);
                }
            }
            Entry::Standard(s) => items.push(Item::Standard(*s)),
            Entry::Section(section) => {
                items.extend(section.rows().into_iter().filter_map(|row| {
                    let c = commands.iter().find(|c| c.id == row.command)?;
                    Some(Item::Live(if c.enabled {
                        row
                    } else {
                        let reason = c.disabled_reason.clone().unwrap_or_default();
                        row.disabled(reason)
                    }))
                }));
            }
            Entry::Submenu(menu) => {
                let inner = resolve_entries(&menu.entries, commands);
                if !inner.is_empty() {
                    items.push(Item::Submenu(Resolved {
                        title: menu.title.clone(),
                        items: inner,
                    }));
                }
            }
        }
    }
    while items.last() == Some(&Item::Separator) {
        items.pop();
    }
    items
}

/// What picking an item does: run `command`, with `args` (JSON) when it
/// has them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pick {
    pub command: String,
    pub args: Option<String>,
}

/// A clickable row, as a backend draws it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    /// `⌘W`, as commands show theirs; empty for none.
    pub shortcut: String,
    pub enabled: bool,
    /// `Some` for a toggle: ticked when on.
    pub checked: Option<bool>,
    pub pick: Pick,
}

/// What a numbered node is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    /// The bar itself (node 0).
    Root,
    Menu {
        title: String,
    },
    Row(Row),
    Separator,
    Standard(Standard),
}

/// A resolved bar numbered, for a backend that names items by number.
///
/// (dbusmenu.) The root is 0, the rest are numbered in order, depth first.
/// A rebuilt tree numbers again from scratch: a backend tells its host the
/// layout changed when the tree is not the one it had.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub id: i32,
    pub kind: Kind,
    pub children: Vec<Self>,
}

impl Tree {
    /// `menus` numbered.
    #[must_use]
    pub fn new(menus: &[Resolved]) -> Self {
        let mut next: i32 = 0;
        let mut number = || {
            next = next.saturating_add(1);
            next
        };
        let children = menus.iter().map(|m| menu_node(m, &mut number)).collect();
        Self {
            id: 0,
            kind: Kind::Root,
            children,
        }
    }

    /// The node numbered `id`.
    #[must_use]
    pub fn find(&self, id: i32) -> Option<&Self> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    /// What picking node `id` does; `None` when it is not a row, or
    /// cannot run now.
    #[must_use]
    pub fn pick(&self, id: i32) -> Option<&Pick> {
        match &self.find(id)?.kind {
            Kind::Row(row) if row.enabled => Some(&row.pick),
            _ => None,
        }
    }

    /// Every node, this one first, depth first.
    pub fn walk(&self) -> Box<dyn Iterator<Item = &Self> + '_> {
        Box::new(std::iter::once(self).chain(self.children.iter().flat_map(Self::walk)))
    }
}

fn menu_node(menu: &Resolved, number: &mut impl FnMut() -> i32) -> Tree {
    let id = number();
    let children = menu
        .items
        .iter()
        .map(|item| match item {
            Item::Submenu(inner) => menu_node(inner, number),
            Item::Command(c) => Tree {
                id: number(),
                kind: Kind::Row(Row {
                    label: c.label.clone(),
                    shortcut: c.shortcut.clone(),
                    enabled: c.enabled,
                    checked: c.checked,
                    pick: Pick {
                        command: c.id.clone(),
                        args: None,
                    },
                }),
                children: Vec::new(),
            },
            Item::Live(row) => Tree {
                id: number(),
                kind: Kind::Row(Row {
                    label: row.label.clone(),
                    shortcut: String::new(),
                    enabled: row.enabled,
                    checked: row.checked,
                    pick: Pick {
                        command: row.command.clone(),
                        args: row.args.clone(),
                    },
                }),
                children: Vec::new(),
            },
            Item::Separator => Tree {
                id: number(),
                kind: Kind::Separator,
                children: Vec::new(),
            },
            Item::Standard(s) => Tree {
                id: number(),
                kind: Kind::Standard(*s),
                children: Vec::new(),
            },
        })
        .collect();
    Tree {
        id,
        kind: Kind::Menu {
            title: menu.title.clone(),
        },
        children,
    }
}
