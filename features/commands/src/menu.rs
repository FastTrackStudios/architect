//! The menu bar (File, Edit, View, …) as a view of the commands
//! (`docs/content/architecture/app-menus.md`): an app says which commands
//! go in which menu, in what order, grouped how; the labels, the keys and
//! whether each can run come from the commands themselves. A backend draws
//! the resolved bar (the native one on macOS: feature `native`).

use crate::palette::Command;

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

    #[must_use]
    pub fn submenu(mut self, menu: Self) -> Self {
        self.entries.push(Entry::Submenu(menu));
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

    /// Resolved against the commands there are now: each command entry
    /// becomes its label, key and state; a group, its commands; a command
    /// that is not there is left out, and so is a menu left empty (the
    /// app's own menu always stays). Separators never lead, trail or
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
