//! What an app declares about how it lives when its window is not on
//! screen: its Dock menu, its tray item, its badge, whether closing the
//! window keeps it running. The menus are Action List menus
//! ([`Menu`](architect_commands::Menu)), resolved against the commands as
//! the menu bar is, so a toggle is ticked and a command that cannot run is
//! greyed with its reason.

use architect_commands::{Command, Kind, Menu, MenuBar, Row, Tree, without_platform};

/// The Action List entry that brings the window (and the Dock icon) back.
pub const SHOW_WINDOW: &str = "PRESENCE_SHOW_WINDOW";
/// Closes the window and keeps the app running in the menu bar.
pub const HIDE_WINDOW: &str = "PRESENCE_HIDE_WINDOW";
/// Quits for real, a hidden window or not.
pub const QUIT: &str = "PRESENCE_QUIT";
/// The toggle: closing the window keeps the app running.
pub const TOGGLE_BACKGROUND: &str = "PRESENCE_TOGGLE_BACKGROUND";
/// The toggle: the app opens at login, in the background.
pub const TOGGLE_LOGIN: &str = "PRESENCE_TOGGLE_LOGIN";
/// The read-only entry that says where the app is (window, background),
/// its badge and whether it opens at login.
pub const STATE: &str = "PRESENCE_STATE";

/// The setting that keeps the app running when its window closes.
pub const KEEP_RUNNING_SETTING: &str = "app.keep_running";
/// The setting that opens the app at login.
pub const OPEN_AT_LOGIN_SETTING: &str = "app.open_at_login";

/// The argument a login launch is started with: no window, no Dock icon.
pub const BACKGROUND_ARG: &str = "--background";

/// What the Dock icon (or a taskbar button) carries over its corner: a
/// count, or a short text. Bound to the app's state: the app gives the
/// badge it has now each time it draws.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Badge {
    #[default]
    None,
    Count(u64),
    Text(String),
}

impl Badge {
    /// The most a count shows; more reads `999+`.
    pub const MAX_COUNT: u64 = 999;

    /// A count of `n` (none for zero).
    #[must_use]
    pub fn count(n: usize) -> Self {
        Self::Count(u64::try_from(n).unwrap_or(u64::MAX))
    }

    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(text.into())
    }

    /// What the badge says: `None` for no badge (a zero count, an empty
    /// text).
    #[must_use]
    pub fn label(&self) -> Option<String> {
        match self {
            Self::None | Self::Count(0) => None,
            Self::Count(n) if *n > Self::MAX_COUNT => Some(format!("{}+", Self::MAX_COUNT)),
            Self::Count(n) => Some(n.to_string()),
            Self::Text(t) => {
                let t = t.trim();
                (!t.is_empty()).then(|| t.to_owned())
            }
        }
    }
}

/// How hard the Dock asks to be looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    /// One bounce.
    Informational,
    /// Bounces until the app is brought forward.
    Critical,
}

/// Progress on the Dock tile: `0.0`–`1.0`, clamped; a value that is not a
/// number is none.
#[must_use]
pub fn progress(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0))
}

/// The tray item's picture.
///
/// macOS draws a template image (it follows the menu bar's light or
/// dark): an SF Symbol by name, or a PNG drawn in black on clear. Linux
/// names an icon from the theme (`StatusNotifierItem`'s `IconName`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayIcon {
    /// An SF Symbol (`tray.full`): macOS.
    pub symbol: Option<String>,
    /// A template PNG, black on clear, about 18pt tall: macOS, where no
    /// symbol is given or it is not there.
    pub template_png: Option<&'static [u8]>,
    /// An icon theme's name (`mail-unread`): Linux.
    pub icon_name: Option<String>,
}

impl TrayIcon {
    /// An SF Symbol on macOS.
    #[must_use]
    pub fn symbol(name: impl Into<String>) -> Self {
        Self {
            symbol: Some(name.into()),
            ..Self::default()
        }
    }

    /// The icon theme's `name` on Linux.
    #[must_use]
    pub fn icon_name(mut self, name: impl Into<String>) -> Self {
        self.icon_name = Some(name.into());
        self
    }

    /// A template PNG (black on clear) on macOS.
    #[must_use]
    pub const fn template_png(mut self, png: &'static [u8]) -> Self {
        self.template_png = Some(png);
        self
    }
}

/// What clicking the tray item does.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum TrayClick {
    /// Either button opens its menu (the macOS way).
    #[default]
    Menu,
    /// The primary button runs the command (show the window), the
    /// secondary opens the menu (the Windows and many a Linux tray's way).
    Run(String),
}

/// The menu-bar (macOS) or tray (Linux, Windows) item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tray {
    pub icon: TrayIcon,
    /// Said on hover.
    pub tooltip: String,
    /// Text beside the icon (a count); `None` for the icon alone.
    pub title: Option<String>,
    pub menu: Menu,
    pub click: TrayClick,
}

impl Tray {
    #[must_use]
    pub const fn new(icon: TrayIcon, menu: Menu) -> Self {
        Self {
            icon,
            tooltip: String::new(),
            title: None,
            menu,
            click: TrayClick::Menu,
        }
    }

    #[must_use]
    pub fn tooltip(mut self, tip: impl Into<String>) -> Self {
        self.tooltip = tip.into();
        self
    }

    /// Text beside the icon; an empty one is none.
    #[must_use]
    pub fn title(mut self, title: Option<String>) -> Self {
        self.title = title.filter(|t| !t.trim().is_empty());
        self
    }

    #[must_use]
    pub fn click(mut self, click: TrayClick) -> Self {
        self.click = click;
        self
    }
}

/// How an app lives when its window is not on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Presence {
    /// The bundle's identifier (`app.fasttrackstudio.task`): the login
    /// item's label, the instance socket's name.
    pub id: String,
    /// The app's name, as the person reads it (`Task`).
    pub name: String,
    /// What right-clicking the Dock icon lists, before the platform's own
    /// items; `None` for those alone.
    pub dock_menu: Option<Menu>,
    /// The menu-bar or tray item; `None` for none.
    pub tray: Option<Tray>,
    /// Whether closing the window may keep the app running (the person
    /// says, in Settings); `None` when the app always quits.
    pub keep_running: Option<bool>,
    /// Whether the app may open at login (the person says, in Settings).
    pub login: bool,
}

impl Presence {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            dock_menu: None,
            tray: None,
            keep_running: None,
            login: false,
        }
    }

    #[must_use]
    pub fn dock_menu(mut self, menu: Menu) -> Self {
        self.dock_menu = Some(menu);
        self
    }

    #[must_use]
    pub fn tray(mut self, tray: Tray) -> Self {
        self.tray = Some(tray);
        self
    }

    /// Closing the window keeps the app running in the menu bar, unless the
    /// person turns it off (on from the start when `on`).
    #[must_use]
    pub const fn background(mut self, on: bool) -> Self {
        self.keep_running = Some(on);
        self
    }

    /// The app may open at login, in the background (off until the person
    /// turns it on).
    #[must_use]
    pub const fn open_at_login(mut self) -> Self {
        self.login = true;
        self
    }

    /// The commands every presence puts on the Action List, which a Dock
    /// or tray menu lists by id.
    #[must_use]
    pub fn standard_menu(&self) -> Menu {
        let mut m = Menu::new(self.name.clone()).command(SHOW_WINDOW);
        if self.keep_running.is_some() {
            m = m.command(HIDE_WINDOW);
        }
        m = m.separator();
        if self.keep_running.is_some() {
            m = m.command(TOGGLE_BACKGROUND);
        }
        if self.login {
            m = m.command(TOGGLE_LOGIN);
        }
        m.separator().command(QUIT)
    }
}

/// A Dock or tray menu, resolved against the commands there are now and
/// numbered: the rows a backend draws and what picking each does.
///
/// The platform's items (Quit, Hide, …) are left out: the Dock adds its
/// own, and a tray quits through [`QUIT`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuTree {
    pub tree: Tree,
}

impl MenuTree {
    /// `menu` resolved against `commands`.
    #[must_use]
    pub fn resolve(menu: &Menu, commands: &[Command]) -> Self {
        let bar = MenuBar {
            menus: vec![menu.clone()],
        };
        Self {
            tree: Tree::new(&without_platform(&bar.resolve(commands))),
        }
    }

    /// The menu's own items (the one menu under the root), each a row, a
    /// separator or a submenu.
    #[must_use]
    pub fn items(&self) -> &[Tree] {
        self.tree
            .children
            .first()
            .map_or(&[], |menu| menu.children.as_slice())
    }

    /// Whether there is nothing to list.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items().is_empty()
    }

    /// The rows, depth first: what a test reads.
    #[must_use]
    pub fn rows(&self) -> Vec<&Row> {
        self.tree
            .walk()
            .filter_map(|n| match &n.kind {
                Kind::Row(row) => Some(row),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Badge, progress};

    #[test]
    fn a_badge_says_a_count_or_a_short_text() {
        assert_eq!(Badge::count(0).label(), None);
        assert_eq!(Badge::count(3).label().as_deref(), Some("3"));
        assert_eq!(Badge::count(1200).label().as_deref(), Some("999+"));
        assert_eq!(Badge::text("  ").label(), None);
        assert_eq!(Badge::text(" new ").label().as_deref(), Some("new"));
        assert_eq!(Badge::None.label(), None);
    }

    #[test]
    fn progress_is_a_fraction_or_none() {
        assert_eq!(progress(Some(1.5)), Some(1.0));
        assert_eq!(progress(Some(-1.0)), Some(0.0));
        assert_eq!(progress(Some(f64::NAN)), None);
        assert_eq!(progress(None), None);
    }
}
