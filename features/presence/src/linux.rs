//! Linux (feature `native`): the tray as a `StatusNotifierItem` (KDE,
//! GNOME with the `AppIndicator` extension, most panels), its menu the
//! commands' dbusmenu object (`architect_commands::DbusMenu`, the menu
//! bar's); the badge, progress and urgency as Unity's `LauncherEntry`
//! (Ubuntu's Dock, KDE's task manager); XDG autostart for opening at login.
//! There is no Dock menu: a launcher's menu is the desktop file's own.
//!
//! The bus runs on a thread of its own. The UI hands it what to show; a
//! click comes back to the window's thread as a [`Heard`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError, mpsc};

use architect_commands::{DbusMenu, DbusMenuState, Kind, Tree};
use dioxus_native::winit::window::UserAttentionType;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

use crate::lifecycle::Effect;
use crate::login::Applied;
use crate::model::{Attention, MenuTree, TrayClick};
use crate::native::WindowOps;
use crate::platform::{Events, Heard, Platform, TrayView};

/// Where the item and its menu are served.
const ITEM_PATH: &str = "/StatusNotifierItem";
const MENU_PATH: &str = "/StatusNotifierItem/Menu";
const LAUNCHER_PATH: &str = "/com/canonical/unity/launcherentry/architect";

/// The tray item as the bus serves it: plain data (the declared menu's
/// live sections stay on the window's thread).
struct Look {
    icon_name: String,
    tooltip: String,
    title: Option<String>,
    is_menu: bool,
    tree: Tree,
}

impl Look {
    fn of(view: &TrayView) -> Self {
        Self {
            icon_name: view.tray.icon.icon_name.clone().unwrap_or_default(),
            tooltip: view.tray.tooltip.clone(),
            title: view.tray.title.clone(),
            is_menu: view.tray.click == TrayClick::Menu,
            tree: root(&view.menu),
        }
    }
}

/// What the UI asks of the bus thread.
enum Ask {
    Tray(Option<Look>),
    Badge(Option<String>),
    Progress(Option<f64>),
    Urgent,
}

/// Linux's presence: the UI's end of the bus thread.
pub struct Linux {
    window: WindowOps,
    asks: Mutex<Option<tokio::sync::mpsc::UnboundedSender<Ask>>>,
}

impl Linux {
    pub const fn new(window: WindowOps) -> Self {
        Self {
            window,
            asks: Mutex::new(None),
        }
    }

    fn ask(&self, ask: Ask) {
        if let Some(tx) = self
            .asks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            let _ = tx.send(ask);
        }
    }
}

impl Platform for Linux {
    fn start(&self, events: Events) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        *self.asks.lock().unwrap_or_else(PoisonError::into_inner) = Some(tx);
        let app = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let spawned = std::thread::Builder::new()
            .name("presence-tray".into())
            .spawn(move || serve(&app, rx, &events));
        if let Err(error) = spawned {
            tracing::warn!(presence.tray = "no_thread", %error, "presence: no tray");
        }
    }

    fn set_dock_menu(&self, _menu: Option<MenuTree>) {}

    fn set_tray(&self, tray: Option<TrayView>) {
        self.ask(Ask::Tray(tray.as_ref().map(Look::of)));
    }

    fn set_badge(&self, label: Option<String>) {
        self.ask(Ask::Badge(label));
    }

    fn set_progress(&self, value: Option<f64>) {
        self.ask(Ask::Progress(value));
    }

    fn request_attention(&self, attention: Attention) {
        if let Some(w) = self.window.os_window() {
            w.request_user_attention(Some(match attention {
                Attention::Informational => UserAttentionType::Informational,
                Attention::Critical => UserAttentionType::Critical,
            }));
        }
        self.ask(Ask::Urgent);
    }

    fn apply(&self, effect: Effect) {
        let _ = self.window.apply(effect);
    }

    fn login_available(&self) -> Result<(), String> {
        crate::login::available()
    }

    fn set_login(&self, id: &str, name: &str, on: bool) -> Result<Applied, String> {
        crate::native::apply_login(id, name, on)
    }
}

/// The bus thread: serves the item while the UI asks, until it is gone.
fn serve(app: &str, asks: tokio::sync::mpsc::UnboundedReceiver<Ask>, events: &Events) {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(error) => {
            tracing::warn!(presence.tray = "no_runtime", %error, "presence: no tray");
            return;
        }
    };
    if let Err(error) = runtime.block_on(run(app, asks, events)) {
        tracing::warn!(
            presence.tray = "not_served",
            %error,
            "presence: no tray (no session bus)"
        );
    }
}

/// The item's state, read by its properties.
struct ItemState {
    id: String,
    tray: Option<Look>,
}

async fn run(
    app: &str,
    mut asks: tokio::sync::mpsc::UnboundedReceiver<Ask>,
    events: &Events,
) -> zbus::Result<()> {
    let connection = zbus::Connection::session().await?;
    let menu = Arc::new(Mutex::new(DbusMenuState {
        tree: Tree::new(&[]),
        revision: 1,
    }));
    let (picked, picks) = mpsc::channel();
    let forward = events.clone();
    // Picks come on a plain channel: hand each to the window's thread.
    let _ = std::thread::Builder::new()
        .name("presence-picks".into())
        .spawn(move || {
            for pick in picks {
                if forward.unbounded_send(Heard::Pick(pick)).is_err() {
                    return;
                }
            }
        });
    let state = Arc::new(Mutex::new(ItemState {
        id: app.to_owned(),
        tray: None,
    }));
    connection
        .object_server()
        .at(MENU_PATH, DbusMenu::new(Arc::clone(&menu), picked))
        .await?;
    connection
        .object_server()
        .at(
            ITEM_PATH,
            Item {
                state: Arc::clone(&state),
                events: events.clone(),
            },
        )
        .await?;
    let name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
    connection.request_name(name.as_str()).await?;
    let menu_emitter = SignalEmitter::new(&connection, MENU_PATH)?;
    let item_emitter = SignalEmitter::new(&connection, ITEM_PATH)?;
    let mut registered = false;
    let mut launcher = Launcher::default();
    while let Some(ask) = asks.recv().await {
        match ask {
            Ask::Tray(tray) => {
                let tree = tray
                    .as_ref()
                    .map_or_else(|| Tree::new(&[]), |t| t.tree.clone());
                let revision = menu
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .set(tree);
                let shown = tray.is_some();
                state.lock().unwrap_or_else(PoisonError::into_inner).tray = tray;
                if let Some(revision) = revision {
                    DbusMenu::announce(&menu_emitter, revision).await?;
                }
                Item::new_title(&item_emitter).await?;
                Item::new_icon(&item_emitter).await?;
                Item::new_tool_tip(&item_emitter).await?;
                if shown && !registered {
                    registered = register(&connection, &name).await;
                    tracing::info!(
                        presence.tray = if registered {
                            "registered"
                        } else {
                            "no_watcher"
                        },
                        "presence: the tray item"
                    );
                }
            }
            Ask::Badge(label) => {
                launcher.count = label.and_then(|l| l.trim_end_matches('+').parse().ok());
                launcher.update(&connection, app).await?;
            }
            Ask::Progress(value) => {
                launcher.progress = value;
                launcher.update(&connection, app).await?;
            }
            Ask::Urgent => {
                launcher.urgent = true;
                launcher.update(&connection, app).await?;
                launcher.urgent = false;
            }
        }
    }
    Ok(())
}

/// The tray menu as dbusmenu wants it: its items straight under the root.
fn root(menu: &MenuTree) -> Tree {
    Tree {
        id: 0,
        kind: Kind::Root,
        children: menu.items().to_vec(),
    }
}

/// Tells the panel's watcher the item is here; whether one took it.
async fn register(connection: &zbus::Connection, name: &str) -> bool {
    connection
        .call_method(
            Some("org.kde.StatusNotifierWatcher"),
            "/StatusNotifierWatcher",
            Some("org.kde.StatusNotifierWatcher"),
            "RegisterStatusNotifierItem",
            &(name,),
        )
        .await
        .is_ok()
}

/// Unity's launcher entry: the count, progress and urgency on the app's
/// launcher, found by its desktop file.
#[derive(Default)]
struct Launcher {
    count: Option<i64>,
    progress: Option<f64>,
    urgent: bool,
}

impl Launcher {
    async fn update(&self, connection: &zbus::Connection, app: &str) -> zbus::Result<()> {
        let mut props: HashMap<&str, Value<'_>> = HashMap::new();
        props.insert("count", Value::from(self.count.unwrap_or(0)));
        props.insert("count-visible", Value::from(self.count.is_some()));
        props.insert("progress", Value::from(self.progress.unwrap_or(0.0)));
        props.insert("progress-visible", Value::from(self.progress.is_some()));
        props.insert("urgent", Value::from(self.urgent));
        connection
            .emit_signal(
                None::<&str>,
                LAUNCHER_PATH,
                "com.canonical.Unity.LauncherEntry",
                "Update",
                &(format!("application://{app}.desktop"), props),
            )
            .await
    }
}

/// The `StatusNotifierItem` a panel reads.
struct Item {
    state: Arc<Mutex<ItemState>>,
    events: Events,
}

impl Item {
    fn with<T>(&self, f: impl FnOnce(&ItemState) -> T) -> T {
        f(&self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }
}

/// An icon as pixels (`a(iiay)`): none, the theme's name is given.
type Pixmaps = Vec<(i32, i32, Vec<u8>)>;

#[expect(
    clippy::unused_self,
    clippy::missing_const_for_fn,
    reason = "the D-Bus interface's shape: zbus calls through `self`"
)]
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    /// The primary button: the item's command, when it has one.
    fn activate(&self, x: i32, y: i32) {
        // Where it was clicked: nothing a click here needs.
        let _ = (x, y);
        let _ = self.events.unbounded_send(Heard::TrayClicked);
    }

    fn secondary_activate(&self, x: i32, y: i32) {
        let _ = (x, y);
    }

    /// The panel shows the menu itself (`Menu`).
    fn context_menu(&self, x: i32, y: i32) {
        let _ = (x, y);
    }

    fn scroll(&self, delta: i32, orientation: &str) {
        let _ = (delta, orientation);
    }

    #[zbus(property)]
    fn category(&self) -> String {
        "ApplicationStatus".to_owned()
    }

    #[zbus(property)]
    fn id(&self) -> String {
        self.with(|s| s.id.clone())
    }

    #[zbus(property)]
    fn title(&self) -> String {
        self.with(|s| {
            s.tray
                .as_ref()
                .map(|t| {
                    t.title.as_ref().map_or_else(
                        || t.tooltip.clone(),
                        |title| format!("{} ({title})", t.tooltip),
                    )
                })
                .unwrap_or_default()
        })
    }

    #[zbus(property)]
    fn status(&self) -> String {
        self.with(|s| {
            if s.tray.is_some() {
                "Active"
            } else {
                "Passive"
            }
            .to_owned()
        })
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        self.with(|s| {
            s.tray
                .as_ref()
                .map(|t| t.icon_name.clone())
                .unwrap_or_default()
        })
    }

    #[zbus(property)]
    fn icon_pixmap(&self) -> Pixmaps {
        Vec::new()
    }

    #[zbus(property)]
    fn tool_tip(&self) -> (String, Pixmaps, String, String) {
        self.with(|s| {
            let (tip, title) = s.tray.as_ref().map_or_else(Default::default, |t| {
                (t.tooltip.clone(), t.title.clone().unwrap_or_default())
            });
            (String::new(), Vec::new(), tip, title)
        })
    }

    /// Either button opens the menu, unless a click runs a command.
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        self.with(|s| s.tray.as_ref().is_none_or(|t| t.is_menu))
    }

    #[zbus(property)]
    fn menu(&self) -> OwnedValue {
        ObjectPath::try_from(MENU_PATH)
            .ok()
            .and_then(|p| OwnedValue::try_from(Value::from(p)).ok())
            .unwrap_or_else(|| OwnedValue::from(false))
    }

    #[zbus(signal)]
    async fn new_title(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_icon(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn new_tool_tip(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
